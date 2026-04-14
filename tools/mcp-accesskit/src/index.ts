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
 *   ax_watch          — Stream accessibility events (value/focus changes)
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { execFile as execFileCb } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFile = promisify(execFileCb);

const __dirname = dirname(fileURLToPath(import.meta.url));
// In dev (tsx): src/index.ts → ax-bridge is in parent dir
// In prod (compiled): dist/index.js → ax-bridge is in parent dir
const AX_BRIDGE = join(__dirname, "..", "ax-bridge");

async function runBridge(args: string[]): Promise<unknown> {
  try {
    const { stdout } = await execFile(AX_BRIDGE, args, {
      timeout: 15_000,
      encoding: "utf-8",
      maxBuffer: 5 * 1024 * 1024, // 5MB for large trees
    });
    return JSON.parse(stdout);
  } catch (err: unknown) {
    if (err instanceof Error && "stderr" in err) {
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
async function resolvePid(pid?: number, appName?: string): Promise<number> {
  if (pid) return pid;
  if (!appName) throw new Error("Either pid or app_name is required");

  const apps = (await runBridge(["list-apps"])) as Array<{
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
    const apps = await runBridge(["list-apps"]);
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
    const resolvedPid = await resolvePid(pid, app_name);
    const tree = await runBridge([
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
    const resolvedPid = await resolvePid(pid, app_name);
    const args = ["find", String(resolvedPid)];
    if (role) args.push("--role", role);
    if (title) args.push("--title", title);
    if (value) args.push("--value", value);

    const results = await runBridge(args);
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
    const resolvedPid = await resolvePid(pid, app_name);
    const result = await runBridge([
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
    const resolvedPid = await resolvePid(pid, app_name);
    const result = await runBridge([
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
    const resolvedPid = await resolvePid(pid, app_name);
    const result = await runBridge([
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
    const resolvedPid = await resolvePid(pid, app_name);
    const result = await runBridge([
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

server.tool(
  "ax_screenshot",
  "Capture a screenshot of the application window as PNG. Use this for canvas-based widgets (plots, images, camera stream) that are invisible to the AccessKit tree. Returns the file path, dimensions, and size.",
  {
    pid: z.number().optional().describe("Process ID of the target app"),
    app_name: z
      .string()
      .optional()
      .describe("App name substring (alternative to PID)"),
    output: z
      .string()
      .default("/tmp/ax-screenshot.png")
      .describe("Output file path for the PNG screenshot"),
  },
  async ({ pid, app_name, output }) => {
    const resolvedPid = await resolvePid(pid, app_name);
    const result = await runBridge([
      "screenshot",
      String(resolvedPid),
      "--output",
      output,
    ]);
    return {
      content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
    };
  }
);

server.tool(
  "ax_app_status",
  "Get the current status of a running GUI application. Returns: running state, window count, connection state (connected/disconnected/reconnecting), and device count. Use to check if the app is ready for interaction.",
  {
    pid: z.number().optional().describe("Process ID of the target app"),
    app_name: z
      .string()
      .optional()
      .describe("App name substring (alternative to PID)"),
  },
  async ({ pid, app_name }) => {
    const resolvedPid = await resolvePid(pid, app_name);
    const result = await runBridge(["app-status", String(resolvedPid)]);
    return {
      content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
    };
  }
);

server.tool(
  "ax_launch",
  "Launch the DAQ GUI application. Starts the rust-daq-gui binary with optional daemon URL and runtime mode. Returns the PID of the launched process. After launching, use ax_app_status to poll until the app is ready.",
  {
    path: z
      .string()
      .describe("Path to the rust-daq-gui binary"),
    daemon_url: z
      .string()
      .optional()
      .describe("Daemon URL to connect to (e.g., http://127.0.0.1:50051)"),
    runtime_mode: z
      .enum(["mock", "native", "universal", "hybrid-db"])
      .optional()
      .describe("Runtime mode for auto-started daemon"),
  },
  async ({ path, daemon_url, runtime_mode }) => {
    const args = ["launch", "--path", path];
    if (daemon_url) args.push("--daemon-url", daemon_url);
    if (runtime_mode) args.push("--runtime-mode", runtime_mode);
    const result = await runBridge(args);
    return {
      content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
    };
  }
);

server.tool(
  "ax_watch",
  "Watch for accessibility events from an application. Listens for value changes, focus changes, and element destruction for the specified duration. Returns all events as a JSON array. Use to observe state changes after triggering an action (e.g., click a button, then watch for value updates).",
  {
    pid: z.number().optional().describe("Process ID of the target app"),
    app_name: z
      .string()
      .optional()
      .describe("App name substring (alternative to PID)"),
    timeout: z
      .number()
      .default(5)
      .describe("How long to watch in seconds (default: 5)"),
    notifications: z
      .string()
      .default("value,focus")
      .describe("Comma-separated notification types: value, focus, destroyed, created, moved, resized, title"),
  },
  async ({ pid, app_name, timeout, notifications }) => {
    const resolvedPid = await resolvePid(pid, app_name);
    // Watch command streams JSONL, so we need to parse each line
    try {
      const { stdout } = await execFile(
        AX_BRIDGE,
        [
          "watch",
          String(resolvedPid),
          "--timeout",
          String(timeout),
          "--notifications",
          notifications,
        ],
        {
          timeout: (timeout + 5) * 1000,
          encoding: "utf-8",
          maxBuffer: 1024 * 1024,
        }
      );
      const events = stdout
        .split("\n")
        .filter((line: string) => line.trim())
        .map((line: string) => {
          try {
            return JSON.parse(line);
          } catch (e) {
            return { raw: line, parse_error: e instanceof Error ? e.message : String(e) };
          }
        });
      return {
        content: [{ type: "text", text: JSON.stringify(events, null, 2) }],
      };
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      return {
        content: [{ type: "text", text: JSON.stringify({ error: msg }) }],
      };
    }
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
