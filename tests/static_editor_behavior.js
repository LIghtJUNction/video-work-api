"use strict";

const assert = require("node:assert/strict");
const {
  lineMetrics,
  buildAction,
  remoteRevisionDecision,
  classifyApiFailure,
} = require("../static/editor.js");

assert.deepEqual(lineMetrics("one\ntwo\nthree", 7), {
  line: 2,
  column: 4,
  lineCount: 3,
});
assert.deepEqual(lineMetrics("", 0), { line: 1, column: 1, lineCount: 1 });

assert.deepEqual(
  buildAction("write_file", {
    project: "aurora-launch",
    path: "project.vpe",
    content: "project \"Aurora\" {}\n",
    expected_revision: 7,
  }),
  {
    action: "write_file",
    project: "aurora-launch",
    path: "project.vpe",
    content: "project \"Aurora\" {}\n",
    expected_revision: 7,
  },
);
assert.deepEqual(buildAction("cancel_job", { job_id: "job-1" }), {
  action: "cancel_job",
  job_id: "job-1",
});

assert.deepEqual(
  remoteRevisionDecision(
    { project: "aurora-launch", revision: 7, dirty: true },
    { slug: "aurora-launch", revision: 8 },
  ),
  { reload: false, remoteAvailable: true },
);
assert.deepEqual(
  remoteRevisionDecision(
    { project: "aurora-launch", revision: 7, dirty: false },
    { slug: "aurora-launch", revision: 8 },
  ),
  { reload: true, remoteAvailable: false },
);
assert.deepEqual(
  remoteRevisionDecision(
    { project: "aurora-launch", revision: 8, dirty: false },
    { slug: "founder-story", revision: 9 },
  ),
  { reload: false, remoteAvailable: false },
);

assert.equal(classifyApiFailure(409, "editor_conflict"), "revision_conflict");
assert.equal(classifyApiFailure(401, "authentication_required"), "authentication_required");
assert.equal(classifyApiFailure(503, "internal_error"), "service_unavailable");

console.log("static editor behavior tests passed");
