use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::studio::{Studio, StudioError};
use crate::{MAX_TEXT_LENGTH, VERSION};

fn mcp_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn mcp_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() }
    })
}

fn tool_result(structured: Value, text: Option<String>) -> Value {
    let body = text.unwrap_or_else(|| {
        serde_json::to_string_pretty(&structured).unwrap_or_else(|_| "{}".into())
    });
    json!({
        "content": [{ "type": "text", "text": body }],
        "structuredContent": structured,
    })
}

pub fn tool_specs() -> Value {
    json!([
        {
            "name": "get_status",
            "description": "Return Video Work API readiness, model, FunClip, and MCP status.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "list_speakers",
            "description": "List speakers and their voice profiles.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "create_speaker",
            "description": "Create a speaker entry in the local voice library.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
                "additionalProperties": false
            }
        },
        {
            "name": "delete_speaker",
            "description": "Delete a speaker that has no profiles.",
            "inputSchema": {
                "type": "object",
                "properties": { "speaker_id": { "type": "string" } },
                "required": ["speaker_id"],
                "additionalProperties": false
            }
        },
        {
            "name": "rename_speaker",
            "description": "Rename a speaker (1–100 characters). Fails if another speaker already uses the name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "speaker_id": { "type": "string" },
                    "name": { "type": "string" }
                },
                "required": ["speaker_id", "name"],
                "additionalProperties": false
            }
        },
        {
            "name": "add_voice_profile",
            "description": "Import reference audio from VWA_REFERENCE_INPUT_DIR. Requires confirm_rights=true and an exact transcript.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "speaker_id": { "type": "string" },
                    "style_name": { "type": "string" },
                    "prompt_text": { "type": "string" },
                    "audio_path": { "type": "string" },
                    "confirm_rights": { "type": "boolean" }
                },
                "required": ["speaker_id", "style_name", "prompt_text", "audio_path", "confirm_rights"],
                "additionalProperties": false
            }
        },
        {
            "name": "delete_voice_profile",
            "description": "Delete a voice profile that has no generation history.",
            "inputSchema": {
                "type": "object",
                "properties": { "profile_id": { "type": "string" } },
                "required": ["profile_id"],
                "additionalProperties": false
            }
        },
        {
            "name": "rename_voice_profile",
            "description": "Rename a voice profile style (1–100 characters). Must be unique per speaker.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "profile_id": { "type": "string" },
                    "style_name": { "type": "string" }
                },
                "required": ["profile_id", "style_name"],
                "additionalProperties": false
            }
        },
        {
            "name": "generate_speech",
            "description": "Zero-shot CosyVoice3 speech using an existing profile. Returns generation id and local audio_path (not base64).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "speaker_id": { "type": "string" },
                    "profile_id": { "type": "string" },
                    "target_text": { "type": "string" },
                    "speed": { "type": "number", "minimum": 0.75, "maximum": 1.25 }
                },
                "required": ["speaker_id", "profile_id", "target_text"],
                "additionalProperties": false
            }
        },
        {
            "name": "get_generation",
            "description": "Get generation status and audio path when complete.",
            "inputSchema": {
                "type": "object",
                "properties": { "generation_id": { "type": "string" } },
                "required": ["generation_id"],
                "additionalProperties": false
            }
        },
        {
            "name": "extract_video_subtitles",
            "description": "Extract precise time-coded subtitles from a video under VWA_VIDEO_INPUT_DIR using FunClip stage-1 ASR.",
            "inputSchema": {
                "type": "object",
                "properties": { "video_path": { "type": "string" } },
                "required": ["video_path"],
                "additionalProperties": false
            }
        }
    ])
}

#[derive(Debug, Deserialize)]
struct CreateSpeakerArgs {
    name: String,
}

#[derive(Debug, Deserialize)]
struct DeleteSpeakerArgs {
    speaker_id: String,
}

#[derive(Debug, Deserialize)]
struct RenameSpeakerArgs {
    speaker_id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct ProfileArgs {
    speaker_id: String,
    style_name: String,
    prompt_text: String,
    audio_path: String,
    confirm_rights: bool,
}

#[derive(Debug, Deserialize)]
struct DeleteProfileArgs {
    profile_id: String,
}

#[derive(Debug, Deserialize)]
struct RenameProfileArgs {
    profile_id: String,
    style_name: String,
}

#[derive(Debug, Deserialize)]
struct GenerationArgs {
    speaker_id: String,
    profile_id: String,
    target_text: String,
    #[serde(default = "default_speed")]
    speed: f64,
}

fn default_speed() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
struct GenerationIdArgs {
    generation_id: String,
}

#[derive(Debug, Deserialize)]
struct SubtitleArgs {
    video_path: String,
}

pub enum McpResponse {
    Json(Value),
    Accepted,
}

impl IntoResponse for McpResponse {
    fn into_response(self) -> Response {
        match self {
            Self::Json(v) => Json(v).into_response(),
            Self::Accepted => StatusCode::ACCEPTED.into_response(),
        }
    }
}

pub fn handle_mcp_message(studio: &Studio, message: Value) -> McpResponse {
    let request_id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(|m| m.as_str()).unwrap_or("");

    match method {
        "initialize" => {
            let protocol = message
                .pointer("/params/protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2025-03-26");
            McpResponse::Json(mcp_ok(
                request_id,
                json!({
                    "protocolVersion": protocol,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "video-work-api",
                        "version": VERSION,
                    }
                }),
            ))
        }
        "notifications/initialized" => McpResponse::Accepted,
        "tools/list" => McpResponse::Json(mcp_ok(request_id, json!({ "tools": tool_specs() }))),
        "tools/call" => {
            let params = message.get("params").cloned().unwrap_or(json!({}));
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            if !arguments.is_object() {
                return McpResponse::Json(mcp_error(
                    request_id,
                    -32602,
                    "arguments must be an object",
                ));
            }
            match call_tool(studio, name, arguments) {
                Ok(result) => McpResponse::Json(mcp_ok(request_id, result)),
                Err(McpToolError::Unknown) => {
                    McpResponse::Json(mcp_error(request_id, -32601, "Unknown tool"))
                }
                Err(McpToolError::InvalidArgs(msg)) => {
                    McpResponse::Json(mcp_error(request_id, -32602, msg))
                }
                Err(McpToolError::NotFound(msg)) => {
                    McpResponse::Json(mcp_error(request_id, -32004, msg))
                }
                Err(McpToolError::Conflict(msg)) => {
                    McpResponse::Json(mcp_error(request_id, -32009, msg))
                }
                Err(McpToolError::Failed(msg)) => {
                    McpResponse::Json(mcp_error(request_id, -32000, msg))
                }
            }
        }
        _ => McpResponse::Json(mcp_error(request_id, -32601, "Unsupported MCP method")),
    }
}

enum McpToolError {
    Unknown,
    InvalidArgs(String),
    NotFound(String),
    Conflict(String),
    Failed(String),
}

fn call_tool(studio: &Studio, name: &str, arguments: Value) -> Result<Value, McpToolError> {
    match name {
        "get_status" => {
            let payload = studio
                .status_payload(true)
                .map_err(|e| McpToolError::Failed(e.to_string()))?;
            Ok(tool_result(payload, None))
        }
        "list_speakers" => {
            let payload = studio
                .list_speakers()
                .map_err(|e| McpToolError::Failed(e.to_string()))?;
            Ok(tool_result(payload, None))
        }
        "create_speaker" => {
            let args: CreateSpeakerArgs = serde_json::from_value(arguments)
                .map_err(|e| McpToolError::InvalidArgs(e.to_string()))?;
            let payload = studio
                .create_speaker(&args.name)
                .map_err(|e| map_studio_err(e))?;
            Ok(tool_result(payload, None))
        }
        "delete_speaker" => {
            let args: DeleteSpeakerArgs = serde_json::from_value(arguments)
                .map_err(|e| McpToolError::InvalidArgs(e.to_string()))?;
            studio
                .delete_speaker(&args.speaker_id)
                .map_err(map_studio_err)?;
            Ok(tool_result(
                json!({ "deleted": true, "speaker_id": args.speaker_id }),
                None,
            ))
        }
        "rename_speaker" => {
            let args: RenameSpeakerArgs = serde_json::from_value(arguments)
                .map_err(|e| McpToolError::InvalidArgs(e.to_string()))?;
            let payload = studio
                .rename_speaker(&args.speaker_id, &args.name)
                .map_err(map_studio_err)?;
            Ok(tool_result(payload, None))
        }
        "add_voice_profile" => {
            let args: ProfileArgs = serde_json::from_value(arguments)
                .map_err(|e| McpToolError::InvalidArgs(e.to_string()))?;
            let payload = studio
                .add_profile_from_sandbox(
                    &args.speaker_id,
                    &args.style_name,
                    &args.prompt_text,
                    &args.audio_path,
                    args.confirm_rights,
                )
                .map_err(map_studio_err)?;
            Ok(tool_result(payload, None))
        }
        "delete_voice_profile" => {
            let args: DeleteProfileArgs = serde_json::from_value(arguments)
                .map_err(|e| McpToolError::InvalidArgs(e.to_string()))?;
            studio
                .delete_profile(&args.profile_id)
                .map_err(map_studio_err)?;
            Ok(tool_result(
                json!({ "deleted": true, "profile_id": args.profile_id }),
                None,
            ))
        }
        "rename_voice_profile" => {
            let args: RenameProfileArgs = serde_json::from_value(arguments)
                .map_err(|e| McpToolError::InvalidArgs(e.to_string()))?;
            let payload = studio
                .rename_profile(&args.profile_id, &args.style_name)
                .map_err(map_studio_err)?;
            Ok(tool_result(payload, None))
        }
        "generate_speech" => {
            let args: GenerationArgs = serde_json::from_value(arguments)
                .map_err(|e| McpToolError::InvalidArgs(e.to_string()))?;
            let text = args.target_text.trim();
            if text.is_empty() || text.len() > MAX_TEXT_LENGTH {
                return Err(McpToolError::InvalidArgs(
                    "Text must contain 1 to 1200 characters".into(),
                ));
            }
            if !(0.75..=1.25).contains(&args.speed) || !args.speed.is_finite() {
                return Err(McpToolError::InvalidArgs(
                    "Speed must be between 0.75 and 1.25".into(),
                ));
            }
            let payload = studio
                .generate_speech(&args.speaker_id, &args.profile_id, text, args.speed)
                .map_err(map_studio_err)?;
            Ok(tool_result(payload, None))
        }
        "get_generation" => {
            let args: GenerationIdArgs = serde_json::from_value(arguments)
                .map_err(|e| McpToolError::InvalidArgs(e.to_string()))?;
            let payload = studio
                .get_generation(&args.generation_id)
                .map_err(map_studio_err)?;
            Ok(tool_result(payload, None))
        }
        "extract_video_subtitles" => {
            let args: SubtitleArgs = serde_json::from_value(arguments)
                .map_err(|e| McpToolError::InvalidArgs(e.to_string()))?;
            let payload = studio
                .extract_subtitles(&args.video_path)
                .map_err(map_studio_err)?;
            let srt = payload
                .get("srt")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(tool_result(payload, Some(srt)))
        }
        _ => Err(McpToolError::Unknown),
    }
}

fn map_studio_err(err: anyhow::Error) -> McpToolError {
    if let Some(se) = err.downcast_ref::<StudioError>() {
        return match se {
            StudioError::SpeakerNotFound
            | StudioError::ProfileNotFound
            | StudioError::GenerationNotFound => McpToolError::NotFound(se.to_string()),
            StudioError::SpeakerHasProfiles
            | StudioError::NameConflict
            | StudioError::ProfileInUse
            | StudioError::ProfileFileInvalid => McpToolError::Conflict(se.to_string()),
            other => McpToolError::Failed(other.to_string()),
        };
    }
    McpToolError::Failed(err.to_string())
}
