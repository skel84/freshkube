// Finds and clicks Freshkube's window for scripts/smoke.sh.
//
//   window id <pid> [front]       the window number, for `screencapture -l`
//   window bounds <pid> [front]   x y width height, in screen points
//   window click <x> <y>          a left click at screen points, as the mouse sends it
//   window scroll <x> <y> <dy>    a scroll wheel at screen points; dy > 0 scrolls down
//
// `front` picks the process's frontmost window instead of its largest, for a
// browser with several windows open.
import CoreGraphics
import Foundation

/// The app's main window: its largest on-screen window. GPUI also keeps a
/// hidden, untitled one. The list runs front to back, so `front` takes the first.
func window(of pid: Int, front: Bool) -> [String: Any]? {
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
    let area = { (w: [String: Any]) -> Double in
        let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
        return (b["Width"] ?? 0) * (b["Height"] ?? 0)
    }
    let own = list.filter {
        $0[kCGWindowOwnerPID as String] as? Int == pid && $0[kCGWindowLayer as String] as? Int == 0 && area($0) > 10_000
    }
    return front ? own.first : own.max { area($0) < area($1) }
}

let args = CommandLine.arguments
guard args.count >= 3 else {
    FileHandle.standardError.write("usage: window id|bounds <pid> [front] | click <x> <y> | scroll <x> <y> <dy>\n".data(using: .utf8)!)
    exit(2)
}
switch args[1] {
case "id", "bounds":
    guard let pid = Int(args[2]), let w = window(of: pid, front: args.count > 3 && args[3] == "front") else { exit(1) }
    if args[1] == "id" {
        print(w[kCGWindowNumber as String]!)
    } else {
        let b = w[kCGWindowBounds as String] as! [String: Double]
        print(Int(b["X"]!), Int(b["Y"]!), Int(b["Width"]!), Int(b["Height"]!))
    }
case "click":
    guard args.count == 4, let x = Double(args[2]), let y = Double(args[3]) else { exit(2) }
    let point = CGPoint(x: x, y: y)
    let source = CGEventSource(stateID: .hidSystemState)
    for (type, pause) in [(CGEventType.mouseMoved, 80_000), (.leftMouseDown, 50_000), (.leftMouseUp, 0)] {
        CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point, mouseButton: .left)!
            .post(tap: .cghidEventTap)
        usleep(useconds_t(pause))
    }
case "scroll":
    guard args.count == 5, let x = Double(args[2]), let y = Double(args[3]), let dy = Int32(args[4]) else { exit(2) }
    let source = CGEventSource(stateID: .hidSystemState)
    CGEvent(mouseEventSource: source, mouseType: .mouseMoved, mouseCursorPosition: CGPoint(x: x, y: y), mouseButton: .left)!
        .post(tap: .cghidEventTap)
    usleep(80_000)
    // In steps, as a trackpad sends them, so lists that animate keep up.
    let steps: Int32 = 10
    for _ in 0..<steps {
        CGEvent(scrollWheelEvent2Source: source, units: .pixel, wheelCount: 1, wheel1: -dy / steps, wheel2: 0, wheel3: 0)!
            .post(tap: .cghidEventTap)
        usleep(16_000)
    }
default:
    exit(2)
}
