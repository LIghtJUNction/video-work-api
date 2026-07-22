const test = require("node:test");
const assert = require("node:assert/strict");
const {
  parseNonEmptyLines,
  parseWholeTextItem,
  shouldShowSingleGenerationAction,
  validateItems,
  runSequential,
} = require("../static/batch-core.js");

test("shows the single-generation action only for multiple items", () => {
  assert.equal(shouldShowSingleGenerationAction(0), false);
  assert.equal(shouldShowSingleGenerationAction(1), false);
  assert.equal(shouldShowSingleGenerationAction(2), true);
  assert.equal(shouldShowSingleGenerationAction(50), true);
});

test("treats a 21-line textarea as one whole generation item", () => {
  const text = Array.from({ length: 21 }, (_, index) => `第 ${index + 1} 行`).join("\n");
  assert.deepEqual(parseWholeTextItem(`  \n${text}\n  `), [text]);
});

test("returns no whole generation item for whitespace-only input", () => {
  assert.deepEqual(parseWholeTextItem(" \n\t\n "), []);
});

test("validates the whole generation item at the 1200-character boundary", () => {
  const accepted = parseWholeTextItem("中".repeat(1200));
  const rejected = parseWholeTextItem("中".repeat(1201));
  assert.equal(validateItems(accepted, { maxItems: 50, maxChars: 1200 }), null);
  assert.deepEqual(validateItems(rejected, { maxItems: 50, maxChars: 1200 }), {
    type: "too_long",
    index: 0,
    count: 1201,
  });
});

test("counts Chinese text by characters at the 1200 boundary", () => {
  const accepted = parseNonEmptyLines("中".repeat(1200));
  const rejected = parseNonEmptyLines("中".repeat(1201));
  assert.equal(validateItems(accepted, { maxItems: 50, maxChars: 1200 }), null);
  assert.deepEqual(validateItems(rejected, { maxItems: 50, maxChars: 1200 }), {
    type: "too_long",
    index: 0,
    count: 1201,
  });
});

test("accepts at most 50 non-empty lines", () => {
  const fifty = Array.from({ length: 50 }, (_, index) => `item ${index}`);
  assert.equal(validateItems(fifty, { maxItems: 50, maxChars: 1200 }), null);
  assert.equal(
    validateItems([...fifty, "overflow"], { maxItems: 50, maxChars: 1200 }).type,
    "too_many",
  );
});

test("enforces the 500-character path boundary", () => {
  assert.equal(
    validateItems(["路".repeat(500)], { maxItems: 50, maxChars: 500 }),
    null,
  );
  assert.deepEqual(
    validateItems(["路".repeat(501)], { maxItems: 50, maxChars: 500 }),
    { type: "too_long", index: 0, count: 501 },
  );
});

test("runs strictly sequentially and continues after a failure", async () => {
  const items = ["a", "b", "c"].map((id) => ({ id }));
  const order = [];
  let active = 0;
  let maxActive = 0;
  await runSequential(items, async (item) => {
    active += 1;
    maxActive = Math.max(maxActive, active);
    order.push(`start:${item.id}`);
    await Promise.resolve();
    active -= 1;
    order.push(`end:${item.id}`);
    if (item.id === "b") throw new Error("expected failure");
    return `result:${item.id}`;
  });
  assert.equal(maxActive, 1);
  assert.deepEqual(order, [
    "start:a", "end:a",
    "start:b", "end:b",
    "start:c", "end:c",
  ]);
  assert.deepEqual(items.map((item) => item.status), ["complete", "failed", "complete"]);
  assert.equal(items[1].error, "expected failure");
});

test("retrying only the failed subset can complete it", async () => {
  const items = ["ok", "retry"].map((id) => ({ id }));
  let firstPass = true;
  await runSequential(items, async (item) => {
    if (firstPass && item.id === "retry") throw new Error("temporary");
    return item.id;
  });
  firstPass = false;
  const failed = items.filter((item) => item.status === "failed");
  assert.deepEqual(failed.map((item) => item.id), ["retry"]);
  await runSequential(failed, async (item) => item.id);
  assert.deepEqual(items.map((item) => item.status), ["complete", "complete"]);
});

test("cancellation after an await does not write back or start the next item", async () => {
  const items = ["first", "second"].map((id) => ({ id }));
  const started = [];
  let cancelled = false;
  let resolveFirst;
  const firstResult = new Promise((resolve) => {
    resolveFirst = resolve;
  });

  const running = runSequential(
    items,
    async (item) => {
      started.push(item.id);
      return firstResult;
    },
    () => {},
    () => cancelled,
  );
  await Promise.resolve();
  cancelled = true;
  resolveFirst("stale result");
  await running;

  assert.deepEqual(started, ["first"]);
  assert.equal(items[0].status, "running");
  assert.equal(items[0].result, undefined);
  assert.equal(items[1].status, "pending");
});
