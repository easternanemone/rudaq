#!/usr/bin/env node
/**
 * mcp-accesskit — MCP server for native GUI interaction via macOS AccessKit.
 *
 * Bridges Claude Code tool calls to macOS AXUIElement accessibility APIs,
 * enabling AI agents to read, query, and interact with native egui/Slint
 * applications the same way claude-in-chrome works for browser-based UIs.
 *
 * Tools:
 *   ax_list_apps      — List GUI applications with PIDs
 *   ax_read_tree      — Dump accessibility widget tree as JSON
 *   ax_find_elements  — Search for elements by role/title/value
 *   ax_click          — Click a button or element by title
 *   ax_set_value      — Set a text field value by nearby label
 *   ax_read_value     — Read element values matching a title
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
// In dev (tsx): src/index.ts → ax-bridge is in parent dir
// In prod (compiled): dist/index.js → ax-bridge is in parent dir
const AX_BRIDGE = join(__dirname, "..", "ax-bridge");

function runBridge(args: string[]): unknown {
  try {
    const stdout = execFileSync(AX_BRIDGE, args, {
      timeout: 15_000,
      encoding: "utf-8",
      maxBuffer: 5 * 1024 * 1024, // 5MB for large trees
    });
    return JSON.parse(stdout);
  } catch (err: unknown) {
    if (err instanceof Error && "status" in err) {
      // execFileSync error with stderr
      const execErr = err as Error & { stderr?: string };
      const stderr = execErr.stderr?.toString().trim() ?? "";
      if (stderr.includes("not found") || stderr.includes("No such file")) {
        throw new Error(
          `ax-bridge binary not found at ${AX_BRIDGE}. Run: cd tools/mcp-accesskit && bash build.sh`
        );
      }
      throw new Error(`ax-bridge error: ${stderr || err.message}`);
    }
    const msg = err instanceof Error ? err.message : String(err);
    throw new Error(`ax-bridge failed: ${msg}`);
  }
}

// Resolve a PID from either a direct PID or an app name
function resolvePid(pid?: number, appName?: string): number {
  if (pid) return pid;
  if (!appName) throw new Error("Either pid or app_name is required");

  const apps = runBridge(["list-apps"]) as Array<{
    name: string;
    pid: number;
    bundleId: string | null;
  }>;
  const match = apps.find(
    (a) =>
      a.name.toLowerCase().includes(appName.toLowerCase()) ||
      (a.bundleId?.toLowerCase().includes(appName.toLowerCase()) ?? false)
  );
  if (!match) {
    const available = apps.map((a) => `${a.name} (${a.pid})`).join(", ");
    throw new Error(
      `No app matching "${appName}". Available: ${available}`
    );
  }
  return match.pid;
}

// Create MCP server
const server = new McpServer({
  name: "mcp-accesskit",
  version: "0.1.0",
});

// --- Tools ---

server.tool(
  "ax_list_apps",
  "List running GUI applications with their PIDs. Use this first to find the target app's PID.",
  {},
  async () => {
    const apps = runBridge(["list-apps"]);
    return {
      content: [{ type: "text", text: JSON.stringify(apps, null, 2) }],
    };
  }
);

server.tool(
  "ax_read_tree",
  "Read the full accessibility widget tree of a native application. Returns a JSON tree with roles, titles, values, actions, and positions for every widget. Use a lower depth for overview, higher for detail.",
  {
    pid: z.number().optional().describe("Process ID of the target app"),
    app_name: z
      .string()
      .optional()
      .describe("App name substring (alternative to PID)"),
    depth: z
      .number()
      .default(6)
      .describe("Maximum tree traversal depth (default: 6)"),
  },
  async ({ pid, app_name, depth }) => {
    const resolvedPid = resolvePid(pid, app_name);
    const tree = runBridge([
      "tree",
      String(resolvedPid),
      "--depth",
      String(depth),
    ]);
    return {
      content: [{ type: "text", text: JSON.stringify(tree, null, 2) }],
    };
  }
);

server.tool(
  "ax_find_elements",
  "Search for accessibility elements matching criteria. Returns matching elements with their roles, titles, values, enabled state, available actions, and screen positions. Title and value matching is case-insensitive substring.",
  {
    pid: z.number().optional().describe("Process ID of the target app"),
    app_name: z
      .string()
      .optional()
      .describe("App name substring (alternative to PID)"),
    role: z
      .string()
      .optional()
      .describe(
        "Accessibility role filter (e.g., AXButton, AXTextField, AXStaticText, AXSlider)"
      ),
    title: z
      .string()
      .optional()
      .describe("Title substring to match (case-insensitive)"),
    value: z
      .string()
      .optional()
      .describe("Value substring to match (case-insensitive)"),
  },
  async ({ pid, app_name, role, title, value }) => {
    const resolvedPid = resolvePid(pid, app_name);
    const args = ["find", String(resolvedPid)];
    if (role) args.push("--role", role);
    if (title) args.push("--title", title);
    if (value) args.push("--value", value);

    const results = runBridge(args);
    return {
      content: [{ type: "text", text: JSON.stringify(results, null, 2) }],
    };
  }
);

server.tool(
  "ax_click",
  "Click a button or interactive element by title. Searches AXButton elements first, then any clickable element. Works for: navigation buttons, device selection, toggle buttons (emission ON/OFF, shutter), start/stop actions, menu items. Returns whether the click succeeded.",
  {
    pid: z.number().optional().describe("Process ID of the target app"),
    app_name: z
      .string()
      .optional()
      .describe("App name substring (alternative to PID)"),
    title: z
      .string()
      .describe("Title substring of the element to click (case-insensitive)"),
  },
  async ({ pid, app_name, title }) => {
    const resolvedPid = resolvePid(pid, app_name);
    const result = runBridge([
      "click",
      String(resolvedPid),
      "--title",
      title,
    ]);
    return {
      content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
    };
  }
);

server.tool(
  "ax_set_value",
  "Set a widget's value by its nearby label. For SLIDERS: sets the numeric value directly (works via AccessKit SetValue action). For TEXT FIELDS: returns a hint to use gRPC SetParameter instead (egui TextEdits cannot accept external input via accessibility APIs). Searches by both title and value attributes.",
  {
    pid: z.number().optional().describe("Process ID of the target app"),
    app_name: z
      .string()
      .optional()
      .describe("App name substring (alternative to PID)"),
    title: z
      .string()
      .describe("Label text near the target field (case-insensitive)"),
    value: z.string().describe("New value to set"),
  },
  async ({ pid, app_name, title, value }) => {
    const resolvedPid = resolvePid(pid, app_name);
    const result = runBridge([
      "set-value",
      String(resolvedPid),
      "--title",
      title,
      "--value",
      value,
    ]);
    return {
      content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
    };
  }
);

server.tool(
  "ax_read_value",
  "Read the value of elements matching a title substring. Returns all matching elements with their roles, titles, and current values. Useful for reading device status, parameter values, connection state, etc.",
  {
    pid: z.number().optional().describe("Process ID of the target app"),
    app_name: z
      .string()
      .optional()
      .describe("App name substring (alternative to PID)"),
    title: z
      .string()
      .describe("Title or label substring to search for (case-insensitive)"),
  },
  async ({ pid, app_name, title }) => {
    const resolvedPid = resolvePid(pid, app_name);
    const result = runBridge([
      "read-value",
      String(resolvedPid),
      "--title",
      title,
    ]);
    return {
      content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
    };
  }
);

server.tool(
  "ax_increment",
  "Increment or decrement a slider/numeric widget by its step size. Finds the slider near the given label and performs AXIncrement or AXDecrement N times. Use for fine adjustments to wavelength, voltage, position, or other numeric parameters.",
  {
    pid: z.number().optional().describe("Process ID of the target app"),
    app_name: z
      .string()
      .optional()
      .describe("App name substring (alternative to PID)"),
    title: z
      .string()
      .describe("Label text near the slider (case-insensitive)"),
    direction: z
      .enum(["increment", "decrement"])
      .default("increment")
      .describe("Direction: increment (increase) or decrement (decrease)"),
    steps: z
      .number()
      .default(1)
      .describe("Number of step increments/decrements to perform (default: 1)"),
  },
  async ({ pid, app_name, title, direction, steps }) => {
    const resolvedPid = resolvePid(pid, app_name);
    const result = runBridge([
      direction,
      String(resolvedPid),
      "--title",
      title,
      "--steps",
      String(steps),
    ]);
    return {
      content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
    };
  }
);

// Start server
async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
