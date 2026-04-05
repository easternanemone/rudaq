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
    guard let value = getAttr(element, kAXPositionAttribute) else { return nil }
    var point = CGPoint.zero
    if AXValueGetValue(value as! AXValue, .cgPoint, &point) {
        return (Double(point.x), Double(point.y))
    }
    return nil
}

func getSize(_ element: AXUIElement) -> (w: Double, h: Double)? {
    guard let value = getAttr(element, kAXSizeAttribute) else { return nil }
    var size = CGSize.zero
    if AXValueGetValue(value as! AXValue, .cgSize, &size) {
        return (Double(size.width), Double(size.height))
    }
    return nil
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
    let value = getStringAttr(element, kAXValueAttribute)
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
    results: inout [FoundElement]
) {
    let elRole = getStringAttr(element, kAXRoleAttribute)
    let elTitle = getStringAttr(element, kAXTitleAttribute)
    let elValue = getStringAttr(element, kAXValueAttribute)
    let elDesc = getStringAttr(element, kAXDescriptionAttribute)

    // Skip menu bar subtree
    if elRole == "AXMenuBar" { return }

    var matches = true
    if let role = role, !role.isEmpty { matches = matches && elRole == role }
    if let title = title, !title.isEmpty { matches = matches && elTitle.localizedCaseInsensitiveContains(title) }
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
            findElements(child, role: role, title: title, value: value, path: path + [i], results: &results)
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

    // Prefer buttons, then any clickable element
    let buttons = results.filter { $0.role == "AXButton" }
    let target = buttons.first ?? results.first

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
    pid: pid_t,
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

struct CommandResult: Codable {
    let success: Bool
    let error: String?
    let data: AnyCodable?
}

// Simple type-erased Codable wrapper
struct AnyCodable: Codable {
    let value: Any

    init(_ value: Any) { self.value = value }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let str = try? container.decode(String.self) { value = str; return }
        if let num = try? container.decode(Double.self) { value = num; return }
        if let bool = try? container.decode(Bool.self) { value = bool; return }
        value = "unsupported"
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch value {
        case let str as String: try container.encode(str)
        case let num as Double: try container.encode(num)
        case let num as Int: try container.encode(num)
        case let bool as Bool: try container.encode(bool)
        case let arr as [Codable]: try container.encode(arr.map { "\($0)" })
        default: try container.encode("\(value)")
        }
    }
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
    let values = results.map { ["role": $0.role, "title": $0.title ?? "", "value": $0.value ?? ""] }
    outputJSON(values)

default:
    outputError("Unknown command: \(command). Available: list-apps, tree, find, click, set-value, read-value")
}
