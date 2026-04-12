#!/usr/bin/env swift
/// AX Bridge — Swift CLI for macOS Accessibility API queries.
/// Called by the mcp-accesskit MCP server to interact with native GUI apps.
///
/// Usage:
///   ax-bridge list-apps
///   ax-bridge tree <pid> [--depth N]
///   ax-bridge find <pid> --role <role> [--title <substring>] [--value <substring>]
///   ax-bridge click <pid> --title <substring>
///   ax-bridge set-value <pid> --title <title> --value <value>
///   ax-bridge read-value <pid> --title <substring>

import Cocoa
import ApplicationServices

// MARK: - Helpers

func getAttr(_ element: AXUIElement, _ attr: String) -> AnyObject? {
    var value: AnyObject?
    AXUIElementCopyAttributeValue(element, attr as CFString, &value)
    return value
}

func getStringAttr(_ element: AXUIElement, _ attr: String) -> String {
    (getAttr(element, attr) as? String) ?? ""
}

func getActions(_ element: AXUIElement) -> [String] {
    var names: CFArray?
    AXUIElementCopyActionNames(element, &names)
    return (names as? [String]) ?? []
}

func getPosition(_ element: AXUIElement) -> (x: Double, y: Double)? {
    guard let raw = getAttr(element, kAXPositionAttribute) else { return nil }
    // AXValue is a CF type — cast always succeeds, but AXValueGetValue
    // fails safely if the type doesn't match .cgPoint
    let value = raw as! AXValue  // swiftlint:disable:this force_cast
    var point = CGPoint.zero
    guard AXValueGetValue(value, .cgPoint, &point) else { return nil }
    return (Double(point.x), Double(point.y))
}

func getSize(_ element: AXUIElement) -> (w: Double, h: Double)? {
    guard let raw = getAttr(element, kAXSizeAttribute) else { return nil }
    let value = raw as! AXValue  // swiftlint:disable:this force_cast
    var size = CGSize.zero
    guard AXValueGetValue(value, .cgSize, &size) else { return nil }
    return (Double(size.width), Double(size.height))
}

/// Resolve the display value for an element, with checkbox numeric fallback.
func resolveValue(_ element: AXUIElement, role: String) -> String {
    let value = getStringAttr(element, kAXValueAttribute)
    if value.isEmpty && role == "AXCheckBox" {
        if let numVal = getAttr(element, kAXValueAttribute) as? Int {
            return numVal == 1 ? "1" : "0"
        }
    }
    return value
}

/// Find the main application window (largest AXStandardWindow by area).
func findMainWindow(_ windows: [AXUIElement]) -> AXUIElement? {
    var mainWin: AXUIElement? = nil
    var maxArea: Double = 0
    for window in windows {
        let subrole = getStringAttr(window, kAXSubroleAttribute)
        if let sz = getSize(window) {
            let area = sz.w * sz.h
            if (subrole == "AXStandardWindow" || mainWin == nil) && area > maxArea {
                maxArea = area
                mainWin = window
            }
        }
    }
    return mainWin
}

/// Normalize whitespace for accessibility title matching (collapses zero-width/non-breaking spaces).
func normalizeWhitespace(_ s: String) -> String {
    s.replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression).trimmingCharacters(in: .whitespaces)
}

// MARK: - Tree Building

struct AXNode: Codable {
    let role: String
    let subrole: String?
    let title: String?
    let value: String?
    let description: String?
    let enabled: Bool
    let actions: [String]
    let position: [Double]?
    let size: [Double]?
    let children: [AXNode]?
}

func buildNode(_ element: AXUIElement, depth: Int, maxDepth: Int) -> AXNode {
    let role = getStringAttr(element, kAXRoleAttribute)
    let subrole = getStringAttr(element, kAXSubroleAttribute)
    let title = getStringAttr(element, kAXTitleAttribute)
    let value = resolveValue(element, role: role)
    let desc = getStringAttr(element, kAXDescriptionAttribute)
    let actions = getActions(element)
    let enabled = (getAttr(element, kAXEnabledAttribute) as? Bool) ?? true
    let pos = getPosition(element)
    let sz = getSize(element)

    // Skip menu bar children to reduce noise
    if role == "AXMenuBar" {
        return AXNode(
            role: role, subrole: nil, title: nil, value: nil,
            description: nil, enabled: true, actions: [],
            position: nil, size: nil, children: nil
        )
    }

    var children: [AXNode]? = nil
    if depth < maxDepth, let axChildren = getAttr(element, kAXChildrenAttribute) as? [AXUIElement] {
        children = axChildren.map { buildNode($0, depth: depth + 1, maxDepth: maxDepth) }
    }

    return AXNode(
        role: role,
        subrole: subrole.isEmpty ? nil : subrole,
        title: title.isEmpty ? nil : title,
        value: value.isEmpty ? nil : (value.count > 500 ? String(value.prefix(500)) + "..." : value),
        description: desc.isEmpty ? nil : desc,
        enabled: enabled,
        actions: actions,
        position: pos != nil ? [pos!.x, pos!.y] : nil,
        size: sz != nil ? [sz!.w, sz!.h] : nil,
        children: children
    )
}

// MARK: - Search

struct FoundElement: Codable {
    let role: String
    let title: String?
    let value: String?
    let description: String?
    let enabled: Bool
    let actions: [String]
    let position: [Double]?
    let size: [Double]?
    let path: [Int]  // Index path from root for re-finding
}

func findElements(
    _ element: AXUIElement,
    role: String?,
    title: String?,
    value: String?,
    path: [Int] = [],
    results: inout [FoundElement],
    normalizedTitle: String? = nil  // Pre-normalized search title (avoids per-node regex)
) {
    let elRole = getStringAttr(element, kAXRoleAttribute)
    let elTitle = getStringAttr(element, kAXTitleAttribute)
    let elValue = resolveValue(element, role: elRole)
    let elDesc = getStringAttr(element, kAXDescriptionAttribute)

    // Skip menu bar subtree
    if elRole == "AXMenuBar" { return }

    // Pre-normalize search title on first call (reused for all recursive calls)
    let normSearch = normalizedTitle ?? title.map { normalizeWhitespace($0) }

    var matches = true
    if let role = role, !role.isEmpty { matches = matches && elRole == role }
    if let normSearch = normSearch, !normSearch.isEmpty {
        let normTitle = normalizeWhitespace(elTitle)
        matches = matches && normTitle.localizedCaseInsensitiveContains(normSearch)
    }
    if let value = value, !value.isEmpty { matches = matches && elValue.localizedCaseInsensitiveContains(value) }

    if matches && (role != nil || title != nil || value != nil) {
        let actions = getActions(element)
        let enabled = (getAttr(element, kAXEnabledAttribute) as? Bool) ?? true
        let pos = getPosition(element)
        let sz = getSize(element)
        results.append(FoundElement(
            role: elRole,
            title: elTitle.isEmpty ? nil : elTitle,
            value: elValue.isEmpty ? nil : (elValue.count > 200 ? String(elValue.prefix(200)) + "..." : elValue),
            description: elDesc.isEmpty ? nil : elDesc,
            enabled: enabled,
            actions: actions,
            position: pos != nil ? [pos!.x, pos!.y] : nil,
            size: sz != nil ? [sz!.w, sz!.h] : nil,
            path: path
        ))
    }

    if let children = getAttr(element, kAXChildrenAttribute) as? [AXUIElement] {
        for (i, child) in children.enumerated() {
            findElements(child, role: role, title: title, value: value, path: path + [i], results: &results, normalizedTitle: normSearch)
        }
    }
}

func navigateToPath(_ root: AXUIElement, path: [Int]) -> AXUIElement? {
    var current = root
    for idx in path {
        guard let children = getAttr(current, kAXChildrenAttribute) as? [AXUIElement],
              idx < children.count else { return nil }
        current = children[idx]
    }
    return current
}

// MARK: - Actions

func findAndPerformAction(
    _ element: AXUIElement,
    title: String,
    action: String = kAXPressAction as String
) -> (success: Bool, elementTitle: String?) {
    var results: [FoundElement] = []
    findElements(element, role: nil, title: title, value: nil, results: &results)

    // Prefer buttons, then popups/checkboxes, then any clickable element
    let buttons = results.filter { $0.role == "AXButton" }
    let popups = results.filter { $0.role == "AXPopUpButton" }
    let checkboxes = results.filter { $0.role == "AXCheckBox" }
    let target = buttons.first ?? popups.first ?? checkboxes.first ?? results.first

    guard let target = target else { return (false, nil) }

    // Re-navigate to the element using its path
    guard let axElement = navigateToPath(element, path: target.path) else { return (false, nil) }
    let result = AXUIElementPerformAction(axElement, action as CFString)
    return (result == .success, target.title ?? target.value)
}

/// Set a widget's value via the appropriate mechanism for its type.
///
/// - **Sliders/SpinButtons**: `AXUIElementSetAttributeValue` works directly because
///   egui processes `Action::SetValue` with `ActionData::NumericValue` on next frame.
/// - **TextEdits**: Cannot be set via AX APIs (egui's AccessKit is push-only for text).
///   Returns a "use_grpc" hint so the agent knows to use `SetParameter` gRPC instead.
func findAndSetValue(
    _ element: AXUIElement,
    pid _: pid_t,
    title: String,
    newValue: String
) -> (success: Bool, elementTitle: String?, hint: String?) {
    // Search by title AND value (egui labels store text in value, not title)
    var labelResults: [FoundElement] = []
    findElements(element, role: nil, title: title, value: nil, results: &labelResults)
    var valueLabelResults: [FoundElement] = []
    findElements(element, role: nil, title: nil, value: title, results: &valueLabelResults)
    let allLabels = labelResults + valueLabelResults

    // Try sliders first — direct SetAttributeValue works
    var allSliders: [FoundElement] = []
    findElements(element, role: "AXSlider", title: nil, value: nil, results: &allSliders)

    for label in allLabels {
        let labelPath = label.path
        for slider in allSliders {
            if slider.path.dropLast() == labelPath.dropLast(),
               let sIdx = slider.path.last, let lIdx = labelPath.last,
               sIdx > lIdx, sIdx - lIdx <= 5 {
                guard let axEl = navigateToPath(element, path: slider.path) else { continue }
                if let numValue = Double(newValue) {
                    let result = AXUIElementSetAttributeValue(axEl, kAXValueAttribute as CFString, numValue as AnyObject)
                    if result == .success {
                        Thread.sleep(forTimeInterval: 0.2)
                        return (true, title, nil)
                    }
                }
            }
        }
    }

    // Try SpinButtons (DragValue) — same SetValue path
    var allSpinButtons: [FoundElement] = []
    findElements(element, role: "AXSpinButton", title: nil, value: nil, results: &allSpinButtons)

    for label in allLabels {
        let labelPath = label.path
        for spin in allSpinButtons {
            if spin.path.dropLast() == labelPath.dropLast(),
               let sIdx = spin.path.last, let lIdx = labelPath.last,
               sIdx > lIdx, sIdx - lIdx <= 5 {
                guard let axEl = navigateToPath(element, path: spin.path) else { continue }
                if let numValue = Double(newValue) {
                    let result = AXUIElementSetAttributeValue(axEl, kAXValueAttribute as CFString, numValue as AnyObject)
                    if result == .success {
                        Thread.sleep(forTimeInterval: 0.2)
                        return (true, title, nil)
                    }
                }
            }
        }
    }

    // Text fields — found but cannot set via AX
    var allFields: [FoundElement] = []
    findElements(element, role: "AXTextField", title: nil, value: nil, results: &allFields)

    for label in allLabels {
        let labelPath = label.path
        for field in allFields {
            if field.path.dropLast() == labelPath.dropLast(),
               let fIdx = field.path.last, let lIdx = labelPath.last,
               fIdx > lIdx, fIdx - lIdx <= 3 {
                return (false, title, "text_field: use gRPC SetParameter to change this value")
            }
        }
    }

    return (false, nil, nil)
}

// MARK: - Commands

struct AppInfo: Codable {
    let name: String
    let pid: Int32
    let bundleId: String?
}

func outputJSON<T: Encodable>(_ value: T) {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    if let data = try? encoder.encode(value), let str = String(data: data, encoding: .utf8) {
        print(str)
    }
}

func outputError(_ msg: String) {
    let err = ["success": false, "error": msg] as [String: Any]
    if let data = try? JSONSerialization.data(withJSONObject: err, options: .prettyPrinted),
       let str = String(data: data, encoding: .utf8) {
        print(str)
    }
    exit(1)
}

// MARK: - Main

let args = CommandLine.arguments
guard args.count >= 2 else {
    outputError("Usage: ax-bridge <command> [args...]")
    exit(1)
}

let command = args[1]

switch command {
case "list-apps":
    let apps = NSWorkspace.shared.runningApplications
        .filter { $0.activationPolicy == .regular }
        .map { AppInfo(name: $0.localizedName ?? "Unknown", pid: $0.processIdentifier, bundleId: $0.bundleIdentifier) }
    outputJSON(apps)

case "tree":
    guard args.count >= 3, let pid = Int32(args[2]) else {
        outputError("Usage: ax-bridge tree <pid> [--depth N]")
        exit(1)
    }
    var maxDepth = 6
    if let idx = args.firstIndex(of: "--depth"), idx + 1 < args.count, let d = Int(args[idx + 1]) {
        maxDepth = d
    }
    let app = AXUIElementCreateApplication(pid)
    let tree = buildNode(app, depth: 0, maxDepth: maxDepth)
    outputJSON(tree)

case "find":
    guard args.count >= 3, let pid = Int32(args[2]) else {
        outputError("Usage: ax-bridge find <pid> [--role <role>] [--title <text>] [--value <text>]")
        exit(1)
    }
    var role: String? = nil
    var title: String? = nil
    var value: String? = nil
    var i = 3
    while i < args.count {
        switch args[i] {
        case "--role": i += 1; role = args[i]
        case "--title": i += 1; title = args[i]
        case "--value": i += 1; value = args[i]
        default: break
        }
        i += 1
    }
    let app = AXUIElementCreateApplication(pid)
    var results: [FoundElement] = []
    findElements(app, role: role, title: title, value: value, results: &results)
    outputJSON(results)

case "click":
    guard args.count >= 3, let pid = Int32(args[2]) else {
        outputError("Usage: ax-bridge click <pid> --title <text>")
        exit(1)
    }
    var title = ""
    if let idx = args.firstIndex(of: "--title"), idx + 1 < args.count { title = args[idx + 1] }
    guard !title.isEmpty else { outputError("--title required"); exit(1) }

    let app = AXUIElementCreateApplication(pid)
    let (success, elTitle) = findAndPerformAction(app, title: title)
    let result: [String: Any] = ["success": success, "clicked": elTitle ?? NSNull()]
    if let data = try? JSONSerialization.data(withJSONObject: result, options: .prettyPrinted),
       let str = String(data: data, encoding: .utf8) {
        print(str)
    }

case "set-value":
    guard args.count >= 3, let pid = Int32(args[2]) else {
        outputError("Usage: ax-bridge set-value <pid> --title <label> --value <value>")
        exit(1)
    }
    var title = "", value = ""
    if let idx = args.firstIndex(of: "--title"), idx + 1 < args.count { title = args[idx + 1] }
    if let idx = args.firstIndex(of: "--value"), idx + 1 < args.count { value = args[idx + 1] }
    guard !title.isEmpty, !value.isEmpty else { outputError("--title and --value required"); exit(1) }

    let app = AXUIElementCreateApplication(pid)
    let (success, elTitle, hint) = findAndSetValue(app, pid: pid, title: title, newValue: value)
    var result: [String: Any] = ["success": success, "field": elTitle ?? NSNull()]
    if let hint = hint { result["hint"] = hint }
    if let data = try? JSONSerialization.data(withJSONObject: result, options: .prettyPrinted),
       let str = String(data: data, encoding: .utf8) {
        print(str)
    }

case "increment", "decrement":
    guard args.count >= 3, let pid = Int32(args[2]) else {
        outputError("Usage: ax-bridge increment|decrement <pid> --title <text> [--steps N]")
        exit(1)
    }
    var title = ""
    var steps = 1
    if let idx = args.firstIndex(of: "--title"), idx + 1 < args.count { title = args[idx + 1] }
    if let idx = args.firstIndex(of: "--steps"), idx + 1 < args.count, let n = Int(args[idx + 1]) { steps = n }
    guard !title.isEmpty else { outputError("--title required"); exit(1) }

    let app = AXUIElementCreateApplication(pid)
    let axAction = command == "increment" ? kAXIncrementAction : kAXDecrementAction

    // Find slider or spinbutton near the label
    var allSliders: [FoundElement] = []
    findElements(app, role: "AXSlider", title: nil, value: nil, results: &allSliders)
    var allSpins: [FoundElement] = []
    findElements(app, role: "AXSpinButton", title: nil, value: nil, results: &allSpins)
    var labels: [FoundElement] = []
    findElements(app, role: nil, title: title, value: nil, results: &labels)
    var valueLabels: [FoundElement] = []
    findElements(app, role: nil, title: nil, value: title, results: &valueLabels)

    var target: AXUIElement? = nil
    let allNumeric = allSliders + allSpins
    let allLabelsFound = labels + valueLabels
    for label in allLabelsFound {
        for widget in allNumeric {
            if widget.path.dropLast() == label.path.dropLast(),
               let wIdx = widget.path.last, let lIdx = label.path.last,
               wIdx > lIdx, wIdx - lIdx <= 5 {
                target = navigateToPath(app, path: widget.path)
                break
            }
        }
        if target != nil { break }
    }

    guard let axElement = target else {
        let result: [String: Any] = ["success": false, "error": "No slider/spinbutton found near '\(title)'"]
        if let data = try? JSONSerialization.data(withJSONObject: result, options: .prettyPrinted),
           let str = String(data: data, encoding: .utf8) { print(str) }
        break
    }

    // Perform action N times
    var lastResult: AXError = .success
    for _ in 0..<steps {
        lastResult = AXUIElementPerformAction(axElement, axAction as CFString)
        if lastResult != .success { break }
        Thread.sleep(forTimeInterval: 0.05)
    }

    Thread.sleep(forTimeInterval: 0.2)
    let numAfter = (getAttr(axElement, kAXValueAttribute) as? Double)
    let result: [String: Any] = [
        "success": lastResult == .success,
        "action": command,
        "steps": steps,
        "value_after": numAfter ?? NSNull()
    ]
    if let data = try? JSONSerialization.data(withJSONObject: result, options: .prettyPrinted),
       let str = String(data: data, encoding: .utf8) { print(str) }

case "read-value":
    guard args.count >= 3, let pid = Int32(args[2]) else {
        outputError("Usage: ax-bridge read-value <pid> --title <text>")
        exit(1)
    }
    var title = ""
    if let idx = args.firstIndex(of: "--title"), idx + 1 < args.count { title = args[idx + 1] }
    guard !title.isEmpty else { outputError("--title required"); exit(1) }

    let app = AXUIElementCreateApplication(pid)
    var results: [FoundElement] = []
    findElements(app, role: nil, title: title, value: nil, results: &results)
    // Also search by value
    var valueResults: [FoundElement] = []
    findElements(app, role: nil, title: nil, value: title, results: &valueResults)
    let combined = results + valueResults
    let values = combined.map { ["role": $0.role, "title": $0.title ?? "", "value": $0.value ?? ""] }
    outputJSON(values)

case "screenshot":
    guard args.count >= 3, let pid = Int32(args[2]) else {
        outputError("Usage: ax-bridge screenshot <pid> --output <path.png>")
        exit(1)
    }
    var outputPath = "/tmp/ax-screenshot.png"
    if let idx = args.firstIndex(of: "--output"), idx + 1 < args.count { outputPath = args[idx + 1] }

    // Get the main application window (largest standard window, not the menu bar)
    let app = AXUIElementCreateApplication(pid)
    guard let windows = getAttr(app, kAXWindowsAttribute) as? [AXUIElement],
          let win = findMainWindow(windows) else {
        let result: [String: Any] = ["success": false, "error": "No standard window found for PID \(pid)"]
        if let data = try? JSONSerialization.data(withJSONObject: result, options: .prettyPrinted),
           let str = String(data: data, encoding: .utf8) { print(str) }
        break
    }

    // Bring window to front for capture
    NSRunningApplication(processIdentifier: pid)?.activate()
    Thread.sleep(forTimeInterval: 0.5)

    // Use screencapture CLI — works on all macOS versions including Sequoia
    // -l <windowid> captures a specific window, but we need the CGWindowID.
    // Simpler: capture the frontmost window with -w (interactive) won't work headless.
    // Instead: capture the full screen region of the window bounds.
    if let pos = getPosition(win), let sz = getSize(win) {
        // -R captures a specific screen region: x,y,w,h
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        proc.arguments = ["-R", "\(Int(pos.x)),\(Int(pos.y)),\(Int(sz.w)),\(Int(sz.h))", "-x", outputPath]
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice
        do {
            try proc.run()
            proc.waitUntilExit()
        } catch {
            let result: [String: Any] = ["success": false, "error": "screencapture failed: \(error.localizedDescription)"]
            if let data = try? JSONSerialization.data(withJSONObject: result, options: .prettyPrinted),
               let str = String(data: data, encoding: .utf8) { print(str) }
            break
        }

        let fm = FileManager.default
        if fm.fileExists(atPath: outputPath),
           let attrs = try? fm.attributesOfItem(atPath: outputPath),
           let size = attrs[.size] as? Int {
            let result: [String: Any] = [
                "success": true,
                "path": outputPath,
                "width": Int(sz.w),
                "height": Int(sz.h),
                "size_bytes": size
            ]
            if let data = try? JSONSerialization.data(withJSONObject: result, options: .prettyPrinted),
               let str = String(data: data, encoding: .utf8) { print(str) }
        } else {
            let result: [String: Any] = ["success": false, "error": "Screenshot file not created"]
            if let data = try? JSONSerialization.data(withJSONObject: result, options: .prettyPrinted),
               let str = String(data: data, encoding: .utf8) { print(str) }
        }
    } else {
        let result: [String: Any] = ["success": false, "error": "Cannot determine window position/size"]
        if let data = try? JSONSerialization.data(withJSONObject: result, options: .prettyPrinted),
           let str = String(data: data, encoding: .utf8) { print(str) }
    }

case "app-status":
    guard args.count >= 3, let pid = Int32(args[2]) else {
        outputError("Usage: ax-bridge app-status <pid>")
        exit(1)
    }
    let running = kill(pid, 0) == 0
    var status: [String: Any] = ["pid": pid, "running": running]

    if running {
        let app = AXUIElementCreateApplication(pid)

        // Check if window exists — find the main standard window for title
        if let windows = getAttr(app, kAXWindowsAttribute) as? [AXUIElement] {
            status["window_count"] = windows.count
            if let win = findMainWindow(windows) {
                status["window_title"] = getStringAttr(win, kAXTitleAttribute)
            }
        }

        // Check for connection status
        var connResults: [FoundElement] = []
        findElements(app, role: nil, title: nil, value: "Connected", results: &connResults)
        var disconnResults: [FoundElement] = []
        findElements(app, role: nil, title: nil, value: "Disconnected", results: &disconnResults)
        var reconnResults: [FoundElement] = []
        findElements(app, role: nil, title: nil, value: "Reconnecting", results: &reconnResults)

        if !connResults.isEmpty {
            status["connection"] = "connected"
        } else if !reconnResults.isEmpty {
            status["connection"] = "reconnecting"
        } else if !disconnResults.isEmpty {
            status["connection"] = "disconnected"
        } else {
            status["connection"] = "unknown"
        }

        // Count devices
        var deviceResults: [FoundElement] = []
        findElements(app, role: nil, title: nil, value: "Loaded", results: &deviceResults)
        for d in deviceResults {
            if let v = d.value, v.contains("devices") {
                status["devices_label"] = v
            }
        }
    }

    if let data = try? JSONSerialization.data(withJSONObject: status, options: [.prettyPrinted, .sortedKeys]),
       let str = String(data: data, encoding: .utf8) { print(str) }

case "launch":
    // Launch the DAQ GUI with specified arguments.
    // Security: only allows launching binaries named "rust-daq-gui" or "rust-daq-daemon".
    var guiPath = ""
    var guiArgs: [String] = []
    if let idx = args.firstIndex(of: "--path"), idx + 1 < args.count { guiPath = args[idx + 1] }
    if let idx = args.firstIndex(of: "--daemon-url"), idx + 1 < args.count {
        guiArgs += ["--daemon-url", args[idx + 1]]
    }
    if let idx = args.firstIndex(of: "--runtime-mode"), idx + 1 < args.count {
        guiArgs += ["--runtime-mode", args[idx + 1]]
    }
    guard !guiPath.isEmpty else { outputError("--path required (path to rust-daq-gui binary)"); exit(1) }

    // Allowlist: only launch known rust-daq binaries
    let allowedNames = ["rust-daq-gui", "rust-daq-daemon"]
    let binaryName = URL(fileURLWithPath: guiPath).lastPathComponent
    guard allowedNames.contains(binaryName) else {
        outputError("Refused to launch '\(binaryName)': only \(allowedNames) are allowed")
        exit(1)
    }

    let fm = FileManager.default
    guard fm.isExecutableFile(atPath: guiPath) else {
        outputError("Binary not found or not executable: \(guiPath)")
        exit(1)
    }

    let proc = Process()
    proc.executableURL = URL(fileURLWithPath: guiPath)
    proc.arguments = guiArgs
    proc.standardOutput = FileHandle.nullDevice
    proc.standardError = FileHandle.nullDevice
    do {
        try proc.run()
        let result: [String: Any] = ["success": true, "pid": proc.processIdentifier]
        if let data = try? JSONSerialization.data(withJSONObject: result, options: .prettyPrinted),
           let str = String(data: data, encoding: .utf8) { print(str) }
    } catch {
        let result: [String: Any] = ["success": false, "error": error.localizedDescription]
        if let data = try? JSONSerialization.data(withJSONObject: result, options: .prettyPrinted),
           let str = String(data: data, encoding: .utf8) { print(str) }
    }

default:
    outputError("Unknown command: \(command). Available: list-apps, tree, find, click, set-value, read-value, increment, decrement, screenshot, app-status, launch")
}
