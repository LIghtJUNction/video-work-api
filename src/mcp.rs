use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::studio::Studio;
use crate::video_editor::{self, EditorError, VideoEditorRequest};
use crate::VERSION;

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
            "name": "video_editor",
            "description": "The sole Video Work API tool for speakers, consent-gated voices, speech/subtitles, virtual project editing, queued media work, gates, lifecycle, and exports.",
            "inputSchema": {
                "oneOf": [
                    action_schema("get_status", json!({}), &[]),
                    action_schema("list_speakers", json!({}), &[]),
                    action_schema(
                        "create_speaker",
                        json!({ "name": { "type": "string" } }),
                        &["name"]
                    ),
                    action_schema(
                        "delete_speaker",
                        json!({ "speaker_id": { "type": "string" } }),
                        &["speaker_id"]
                    ),
                    action_schema(
                        "rename_speaker",
                        json!({
                            "speaker_id": { "type": "string" },
                            "name": { "type": "string" }
                        }),
                        &["speaker_id", "name"]
                    ),
                    action_schema(
                        "add_voice_profile",
                        json!({
                            "speaker_id": { "type": "string" },
                            "style_name": { "type": "string" },
                            "prompt_text": { "type": "string" },
                            "audio_path": { "type": "string" },
                            "confirm_rights": { "type": "boolean" }
                        }),
                        &["speaker_id", "style_name", "prompt_text", "audio_path", "confirm_rights"]
                    ),
                    action_schema(
                        "delete_voice_profile",
                        json!({ "profile_id": { "type": "string" } }),
                        &["profile_id"]
                    ),
                    action_schema(
                        "rename_voice_profile",
                        json!({
                            "profile_id": { "type": "string" },
                            "style_name": { "type": "string" }
                        }),
                        &["profile_id", "style_name"]
                    ),
                    action_schema(
                        "generate_speech",
                        json!({
                            "speaker_id": { "type": "string" },
                            "profile_id": { "type": "string" },
                            "target_text": { "type": "string" },
                            "speed": { "type": "number", "minimum": 0.75, "maximum": 1.25 }
                        }),
                        &["speaker_id", "profile_id", "target_text"]
                    ),
                    action_schema(
                        "get_generation",
                        json!({ "generation_id": { "type": "string" } }),
                        &["generation_id"]
                    ),
                    action_schema(
                        "extract_video_subtitles",
                        json!({ "video_path": { "type": "string" } }),
                        &["video_path"]
                    ),
                    action_schema("list_projects", json!({}), &[]),
                    action_schema(
                        "create_project",
                        json!({
                            "slug": { "type": "string" },
                            "content": { "type": "string" }
                        }),
                        &["slug"]
                    ),
                    action_schema(
                        "get_tree",
                        json!({ "project": { "type": "string" } }),
                        &["project"]
                    ),
                    action_schema(
                        "read_file",
                        json!({
                            "project": { "type": "string" },
                            "path": { "type": "string" }
                        }),
                        &["project", "path"]
                    ),
                    action_schema(
                        "write_file",
                        json!({
                            "project": { "type": "string" },
                            "path": { "const": "project.vpe" },
                            "content": { "type": "string" },
                            "expected_revision": { "type": "integer", "minimum": 1 }
                        }),
                        &["project", "path", "content", "expected_revision"]
                    ),
                    action_schema(
                        "validate",
                        json!({ "project": { "type": "string" } }),
                        &["project"]
                    ),
                    action_schema(
                        "allocate_variant_ids",
                        json!({
                            "namespace": { "type": "string" },
                            "count": { "type": "integer", "minimum": 1, "maximum": 10000 },
                            "languages": {
                                "type": "array",
                                "items": { "type": "string" }
                            }
                        }),
                        &["namespace"]
                    ),
                    action_schema(
                        "export",
                        json!({ "project": { "type": "string" } }),
                        &["project"]
                    ),
                    action_schema(
                        "get_job",
                        json!({ "job_id": { "type": "string" } }),
                        &["job_id"]
                    ),
                    action_schema(
                        "cancel_job",
                        json!({ "job_id": { "type": "string" } }),
                        &["job_id"]
                    ),
                    action_schema(
                        "extract_analysis_frames",
                        json!({
                            "request": {
                                "type": "object",
                                "properties": {
                                    "video_path": { "type": "string" },
                                    "max_frames": { "type": "integer", "minimum": 1, "maximum": 12 },
                                    "resolution": {
                                        "type": "array",
                                        "prefixItems": [
                                            { "type": "integer", "minimum": 1, "maximum": 1920 },
                                            { "type": "integer", "minimum": 1, "maximum": 1920 }
                                        ],
                                        "minItems": 2,
                                        "maxItems": 2
                                    },
                                    "add_timestamp_overlay": { "type": "boolean" },
                                    "asr_segments": { "type": "array", "items": { "type": "object" } }
                                },
                                "required": ["video_path"],
                                "additionalProperties": false
                            }
                        }),
                        &["request"]
                    ),
                    action_schema(
                        "analyze_safe_trims",
                        json!({
                            "request": {
                                "type": "object",
                                "properties": {
                                    "requested_start": { "type": "number", "minimum": 0 },
                                    "requested_end": { "type": "number", "exclusiveMinimum": 0 },
                                    "search_radius": { "type": "number", "minimum": 0 },
                                    "words": { "type": "array", "items": { "type": "object" } },
                                    "video_path": { "type": "string" }
                                },
                                "required": ["video_path", "requested_start", "requested_end"],
                                "additionalProperties": false
                            }
                        }),
                        &["request"]
                    ),
                    action_schema(
                        "validate_phase",
                        json!({
                            "project": { "type": "string" },
                            "request": {
                                "type": "object",
                                "properties": {
                                    "phase": { "enum": ["pre-render", "pre-package", "acceptance"] },
                                    "input_manifest": { "type": "object", "additionalProperties": { "type": "string" } },
                                    "subtitle_overflow": { "type": "object" },
                                    "deliverable_stem": { "type": "string" },
                                    "master_output": { "type": "string" },
                                    "variant_outputs": { "type": "object", "additionalProperties": { "type": "string" } },
                                    "cover_jobs": { "type": "object", "additionalProperties": { "type": "string" } },
                                    "copy_consistency": { "type": "object" },
                                    "job_id": { "type": "string" },
                                },
                                "required": ["phase"],
                                "additionalProperties": false
                            }
                        }),
                        &["project", "request"]
                    ),
                    action_schema(
                        "render_cover",
                        json!({
                            "project": { "type": "string" },
                            "variant_key": { "type": "string" },
                            "spec": {
                                "type": "object",
                                "properties": {
                                    "source_video": { "type": "string" },
                                    "frame_timestamp": { "type": "number", "minimum": 0 },
                                    "layout_profile": {
                                        "enum": ["smoke-glass", "banner-card", "diagonal", "editorial-black-gold"]
                                    },
                                    "title": { "type": "string" },
                                    "subtitle": { "type": "string" }
                                },
                                "required": ["source_video", "frame_timestamp", "layout_profile", "title"],
                                "additionalProperties": false
                            }
                        }),
                        &["project", "variant_key", "spec"]
                    ),
                    action_schema(
                        "cleanup_intermediates",
                        json!({
                            "project": { "type": "string" },
                            "request": {
                                "type": "object",
                                "properties": {
                                    "paths": { "type": "array", "items": { "type": "string" } },
                                    "dry_run": { "type": "boolean", "default": true }
                                },
                                "additionalProperties": false
                            }
                        }),
                        &["project", "request"]
                    ),
                    action_schema(
                        "archive_completed_sources",
                        json!({
                            "project": { "type": "string" },
                            "request": {
                                "type": "object",
                                "properties": {
                                    "deliverable_stem": { "type": "string" },
                                    "dry_run": { "type": "boolean", "default": true }
                                },
                                "required": ["deliverable_stem"],
                                "additionalProperties": false
                            }
                        }),
                        &["project", "request"]
                    )
                ]
            }
        }
    ])
}

fn action_schema(action: &str, properties: Value, required: &[&str]) -> Value {
    let mut fields = properties.as_object().cloned().unwrap_or_default();
    fields.insert("action".into(), json!({ "const": action }));
    let mut required_fields = vec![Value::String("action".into())];
    required_fields.extend(
        required
            .iter()
            .map(|field| Value::String((*field).to_string())),
    );
    json!({
        "type": "object",
        "properties": fields,
        "required": required_fields,
        "additionalProperties": false
    })
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
    if name != "video_editor" {
        return Err(McpToolError::Unknown);
    }
    let request: VideoEditorRequest = serde_json::from_value(arguments)
        .map_err(|error| McpToolError::InvalidArgs(error.to_string()))?;
    let payload = video_editor::execute(studio, request).map_err(map_editor_error)?;
    Ok(tool_result(payload, None))
}

fn map_editor_error(error: EditorError) -> McpToolError {
    match error {
        EditorError::Invalid(message) => McpToolError::InvalidArgs(message),
        EditorError::NotFound(message) => McpToolError::NotFound(message),
        EditorError::Conflict(message) => McpToolError::Conflict(message),
        EditorError::Internal(error) => McpToolError::Failed(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::tool_specs;

    #[test]
    fn only_video_editor_is_exposed() {
        let specs = tool_specs();
        let names: Vec<_> = specs
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert_eq!(names, vec!["video_editor"]);
        let actions = specs[0]["inputSchema"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|schema| schema["properties"]["action"]["const"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            actions,
            [
                "get_status",
                "list_speakers",
                "create_speaker",
                "delete_speaker",
                "rename_speaker",
                "add_voice_profile",
                "delete_voice_profile",
                "rename_voice_profile",
                "generate_speech",
                "get_generation",
                "extract_video_subtitles",
                "list_projects",
                "create_project",
                "get_tree",
                "read_file",
                "write_file",
                "validate",
                "allocate_variant_ids",
                "export",
                "get_job",
                "cancel_job",
                "extract_analysis_frames",
                "analyze_safe_trims",
                "validate_phase",
                "render_cover",
                "cleanup_intermediates",
                "archive_completed_sources",
            ]
        );
    }
}
