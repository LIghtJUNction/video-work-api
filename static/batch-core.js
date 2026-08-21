(function (root, factory) {
  const core = factory();
  if (typeof module === "object" && module.exports) module.exports = core;
  else root.BatchCore = core;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  "use strict";

  function characterCount(value) {
    return Array.from(String(value || "")).length;
  }

  function parseNonEmptyLines(value) {
    return String(value || "")
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
  }

  function parseWholeTextItem(value) {
    const text = String(value || "").trim();
    return text ? [text] : [];
  }

  function shouldShowSingleGenerationAction(itemCount) {
    return itemCount > 1;
  }

  function formatTranscriptionResults(jobs) {
    return Array.from(jobs || [])
      .filter((job) => job?.status === "complete" && job.result)
      .map((job, index) => {
        const label = String(job.label || `Recording ${index + 1}`);
        const text = typeof job.result.text === "string" ? job.result.text : "";
        return `[${index + 1}] ${label}\n${text}`;
      })
      .join("\n\n");
  }

  function validateItems(items, options) {
    const values = Array.from(items || []);
    if (!values.length) return { type: "empty" };
    if (values.length > options.maxItems) {
      return { type: "too_many", count: values.length };
    }
    if (options.maxChars) {
      const index = values.findIndex(
        (value) => characterCount(value) > options.maxChars,
      );
      if (index >= 0) {
        return {
          type: "too_long",
          index,
          count: characterCount(values[index]),
        };
      }
    }
    return null;
  }

  async function runSequential(
    items,
    execute,
    onChange = function () {},
    cancelled = function () { return false; },
  ) {
    for (const item of items) {
      item.status = "pending";
      item.error = "";
      onChange(item);
    }
    for (const item of items) {
      if (cancelled()) break;
      item.status = "running";
      onChange(item);
      try {
        const result = await execute(item);
        if (cancelled()) break;
        item.result = result;
        item.status = "complete";
      } catch (error) {
        if (cancelled()) break;
        item.status = "failed";
        item.error = error instanceof Error ? error.message : String(error);
      }
      onChange(item);
    }
    return items;
  }

  return {
    characterCount,
    parseNonEmptyLines,
    parseWholeTextItem,
    shouldShowSingleGenerationAction,
    formatTranscriptionResults,
    validateItems,
    runSequential,
  };
});
