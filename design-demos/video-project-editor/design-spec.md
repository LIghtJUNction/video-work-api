# Video Project Editor — Design Direction Spec

## Product

Video Project Editor is the primary human and agent interface for Video Work
API. It presents every video project as a virtual folder and every edit as plain
text. The canonical editable file is `project.vpe`; parsers turn its sources,
timeline tracks, clips, cuts, markers, holds, transitions, variants, quality
gates, and export declarations into deterministic Video Work API operations.
Generated folders such as `receipts/`, `exports/`, and `.history/` remain
visible but are read-only. The product must support multiple projects in one
workspace and let a human watch an agent edit in real time.

## Audience and use

The primary user is a technical video operator supervising an AI agent. They
work on a laptop at roughly one metre, often for long sessions. They need to
understand what changed, whether the text parses, what the timeline means, and
whether an export is queued without switching to a separate operations
dashboard. The interface should feel immediately familiar to users of text
editors, not like a simplified consumer NLE.

## Core screen content

- Global product bar: Video Project Editor, connection state, active workspace,
  validation state, and export action.
- Project Explorer: several project folders, expandable files, asset counts,
  read-only generated folders, and per-project dirty/running indicators.
- Tabs and breadcrumbs: multiple virtual files can be inspected while
  `project.vpe` remains the canonical writable source.
- Text editor: line numbers, syntax colour, cursor/selection, agent presence,
  change markers, parser diagnostics, and minimap or document outline.
- Parsed project context: a compact timeline/outline representation for tracks,
  clips, cuts, markers, transitions, variants, and gates. It is an inspection
  aid, never a second source of truth.
- Queue/output area: queued/running/failed/completed jobs, progress, cancel, and
  receipt/export links.
- Status bar: branch/revision, cursor position, encoding, parser version, frame
  rate, duration, and Agent connection.

## Interaction assumptions

- A single MCP tool named `video_editor` accepts an `action` enum for
  `list_projects`, `read_file`, `write_file`, `validate`, `export`,
  `get_job`, and `cancel_job`.
- Writes use optimistic revisions and atomic no-overwrite semantics. The UI
  streams changes and shows the Agent cursor, but only validated revisions can
  export.
- The virtual filesystem is sandboxed. Paths cannot escape a configured project
  root, generated files are immutable through the editor, and every accepted
  edit creates a revision plus an audit receipt.
- The first implementation is desktop-first at 1440 × 900. A compact layout may
  stack panels later; this direction exercise does not design mobile editing.
- All product naming, labels, schema keywords, example projects, and file names
  are English.

## Visual and emotional direction

The design must communicate precision, observability, and calm control. It may
be dense because density carries real project information, but it must not look
like a generic neon AI dashboard. The existing Video Work API uses warm ivory,
ink, clay, muted green, serif display type, and mono technical labels. One
direction should preserve that identity, while the other two deliberately test
more native editor and film-production interpretations. Body text is at least
14px, small metadata at least 12px, and all important text meets 4.5:1 contrast.

## Output and comparison rules

Create three complete, self-contained HTML direction prototypes at a fixed
1440 × 900 viewport, each with the same real project content and a structurally
different layout. Each prototype must visibly include multiple projects, an
expanded virtual folder, `project.vpe` text, line numbers, a parsed timeline or
outline, Agent editing presence, validation state, and an export queue. No stock
imagery is required because this is a document/tool interface; every element
must carry product information.

## Shared sample content

Workspace: `Campaign Studio`

Projects:

- `aurora-launch` — active, modified, one export running
- `founder-story` — valid, idle
- `product-loop` — one parser warning

Canonical document excerpt:

```text
project "Aurora Launch" {
  canvas 1080x1920 @ 30fps
  source host = "assets/host-take-03.mp4"
  source detail = "assets/product-detail.mp4"

  timeline {
    track main {
      clip host source 00:00:00.600..00:00:06.800 at 00:00:00.000
      cut at 00:00:03.120
      clip detail source 00:00:01.400..00:00:05.900 at 00:00:06.200
      transition cross_dissolve at 00:00:06.200 duration 12f
    }
    track overlay {
      hold host at 00:00:02.500..00:00:03.200 source_time 00:00:02.800
    }
  }

  marker "Opening hook" at 00:00:03.000
  variant "ZH-EN" aspect 9:16 subtitles "subs.zh-en.ass"
  gate pre_render require opening_hook, continuous_timeline
}
```

The three layout anchors are:

1. Native Code Workbench — conservative desktop editor grammar with explorer,
   tabs, editor, outline, bottom panel, and familiar status bar.
2. Editorial Production Desk — retains a text editor core but uses the existing
   warm Video Work API identity and a horizontal filmstrip/timeline inspection
   rail.
3. Live Agent Split View — a more operational layout where the text diff and
   parsed project graph are equal peers, with projects and queue visible at all
   times.
