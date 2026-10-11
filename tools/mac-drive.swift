// Drives an app on the Mac's real screen for checking it by hand from a
// script: the pointer, the keyboard, the wheel, and where a process's windows
// are. Needs Accessibility (to post events) and Screen Recording (for
// `screencapture -l`) for the app running it. Build and use:
//
//   swiftc -O tools/mac-drive.swift -o target/mac-drive
//   target/mac-drive windows <pid>            id x y w h title, one per window
//   target/mac-drive click <x> <y> [clicks] [mods]
//   target/mac-drive rclick <x> <y>
//   target/mac-drive move <x> <y>
//   target/mac-drive drag <x1> <y1> <x2> <y2> [slow]   slow: a file drag between apps
//   target/mac-drive scroll <x> <y> <dy> [line|pixel] (dy > 0 scrolls down)
//   target/mac-drive key <keycode> [mods]     mods: cmd,shift,opt,ctrl
//   target/mac-drive type <text>              Unicode, as typed
//   target/mac-drive front <pid>              brings the app forward
//   target/mac-drive crop <in> <out> <x> <y> <w> <h> [zoom]   a region of a PNG, enlarged
//
// Points are global, top-left origin, as CGWindowList reports windows.

import AppKit
import CoreGraphics
import Foundation

let args = CommandLine.arguments
func num(_ i: Int) -> Double { Double(args[i]) ?? 0 }
func flags(_ s: String?) -> CGEventFlags {
    var f: CGEventFlags = []
    for m in (s ?? "").split(separator: ",") {
        switch m {
        case "cmd": f.insert(.maskCommand)
        case "shift": f.insert(.maskShift)
        case "opt", "alt": f.insert(.maskAlternate)
        case "ctrl": f.insert(.maskControl)
        default: break
        }
    }
    return f
}
let src = CGEventSource(stateID: .hidSystemState)
func post(_ e: CGEvent?) { e?.post(tap: .cghidEventTap); usleep(8000) }
func mouse(_ type: CGEventType, _ p: CGPoint, _ button: CGMouseButton = .left, clicks: Int = 1, mods: CGEventFlags = []) {
    let e = CGEvent(mouseEventSource: src, mouseType: type, mouseCursorPosition: p, mouseButton: button)
    e?.setIntegerValueField(.mouseEventClickState, value: Int64(clicks))
    e?.flags = mods
    post(e)
}

guard args.count > 1 else { print("usage: see the top of tools/mac-drive.swift"); exit(2) }
switch args[1] {
case "windows":
    let pid = Int32(args[2]) ?? 0
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    for w in list where (w[kCGWindowOwnerPID as String] as? Int32) == pid {
        let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
        let layer = w[kCGWindowLayer as String] as? Int ?? 0
        print(w[kCGWindowNumber as String] ?? 0, Int(b["X"] ?? 0), Int(b["Y"] ?? 0), Int(b["Width"] ?? 0), Int(b["Height"] ?? 0), "layer=\(layer)", w[kCGWindowName as String] as? String ?? "")
    }
case "click":
    let p = CGPoint(x: num(2), y: num(3))
    let n = args.count > 4 ? Int(args[4]) ?? 1 : 1
    let m = flags(args.count > 5 ? args[5] : nil)
    mouse(.mouseMoved, p)
    for i in 1...n {
        mouse(.leftMouseDown, p, clicks: i, mods: m)
        mouse(.leftMouseUp, p, clicks: i, mods: m)
    }
case "down": // down <x> <y>, dragto <x> <y>, up <x> <y>: a press held across calls, to look mid-drag
    mouse(.mouseMoved, CGPoint(x: num(2), y: num(3)))
    mouse(.leftMouseDown, CGPoint(x: num(2), y: num(3)))
case "dragto":
    mouse(.leftMouseDragged, CGPoint(x: num(2), y: num(3)))
case "up":
    mouse(.leftMouseUp, CGPoint(x: num(2), y: num(3)))
case "rclick":
    let p = CGPoint(x: num(2), y: num(3))
    mouse(.mouseMoved, p)
    mouse(.rightMouseDown, p, .right)
    mouse(.rightMouseUp, p, .right)
case "move":
    mouse(.mouseMoved, CGPoint(x: num(2), y: num(3)))
case "drag": // drag <x1> <y1> <x2> <y2> [slow]: slow holds, crosses the drag threshold and hovers, as a file drag between apps needs
    let a = CGPoint(x: num(2), y: num(3)), b = CGPoint(x: num(4), y: num(5))
    let slow = args.count > 6 && args[6] == "slow"
    mouse(.mouseMoved, a)
    mouse(.leftMouseDown, a)
    if slow {
        usleep(300_000)
        for d in 1...6 { mouse(.leftMouseDragged, CGPoint(x: a.x + Double(d) * 2, y: a.y + Double(d))); usleep(30_000) }
    }
    let steps = slow ? 60 : 20
    for i in 1...steps {
        let t = Double(i) / Double(steps)
        mouse(.leftMouseDragged, CGPoint(x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t))
        if slow { usleep(15_000) }
    }
    if slow { usleep(400_000); mouse(.leftMouseDragged, b); usleep(200_000) }
    mouse(.leftMouseUp, b)
case "scroll":
    mouse(.mouseMoved, CGPoint(x: num(2), y: num(3)))
    let pixel = args.count > 5 && args[5] == "pixel"
    // The wheel's sign: positive is content moving up (scrolling back).
    let e = CGEvent(scrollWheelEvent2Source: src, units: pixel ? .pixel : .line, wheelCount: 1, wheel1: Int32(-num(4)), wheel2: 0, wheel3: 0)
    post(e)
case "key":
    let code = CGKeyCode(Int(args[2]) ?? 0)
    let m = flags(args.count > 3 ? args[3] : nil)
    let d = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: true); d?.flags = m; post(d)
    let u = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: false); u?.flags = m; post(u)
case "type":
    for ch in args[2...].joined(separator: " ").utf16.map({ $0 }) {
        var c = ch
        // No modifiers, whatever the event source last saw.
        let d = CGEvent(keyboardEventSource: src, virtualKey: 0, keyDown: true)
        d?.flags = []
        d?.keyboardSetUnicodeString(stringLength: 1, unicodeString: &c); post(d)
        let u = CGEvent(keyboardEventSource: src, virtualKey: 0, keyDown: false)
        u?.flags = []
        u?.keyboardSetUnicodeString(stringLength: 1, unicodeString: &c); post(u)
    }
case "crop": // crop <in.png> <out.png> <x> <y> <w> <h> [zoom]: a region, enlarged, to look at closely
    let url = URL(fileURLWithPath: args[2])
    guard let s = CGImageSourceCreateWithURL(url as CFURL, nil), let img = CGImageSourceCreateImageAtIndex(s, 0, nil),
          let part = img.cropping(to: CGRect(x: num(4), y: num(5), width: num(6), height: num(7))) else { print("cannot read \(args[2])"); exit(1) }
    let z = args.count > 8 ? num(8) : 1
    let w = Int(Double(part.width) * z), h = Int(Double(part.height) * z)
    let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.interpolationQuality = .none
    ctx.draw(part, in: CGRect(x: 0, y: 0, width: w, height: h))
    let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: args[3]) as CFURL, "public.png" as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, ctx.makeImage()!, nil)
    CGImageDestinationFinalize(dest)
case "ax": // ax <pid>: what VoiceOver sees of the focused element (counts and ranges, never the text)
    let app = AXUIElementCreateApplication(Int32(args[2]) ?? 0)
    var focused: CFTypeRef?
    guard AXUIElementCopyAttributeValue(app, kAXFocusedUIElementAttribute as CFString, &focused) == .success, let el = focused else { print("no focused element"); exit(1) }
    let e = el as! AXUIElement
    func attr(_ name: String) -> CFTypeRef? {
        var v: CFTypeRef?
        return AXUIElementCopyAttributeValue(e, name as CFString, &v) == .success ? v : nil
    }
    func range(_ v: CFTypeRef?) -> String {
        guard let v, CFGetTypeID(v) == AXValueGetTypeID() else { return "-" }
        var r = CFRange(); AXValueGetValue(v as! AXValue, .cfRange, &r); return "\(r.location)+\(r.length)"
    }
    print("role:", attr(kAXRoleAttribute) as? String ?? "-")
    print("label:", (attr(kAXDescriptionAttribute) as? String).map { "\($0.count) chars" } ?? "-")
    print("value:", (attr(kAXValueAttribute) as? String).map { "\($0.utf16.count) units" } ?? "-")
    print("characters:", attr(kAXNumberOfCharactersAttribute) as? Int ?? -1)
    print("selected range:", range(attr(kAXSelectedTextRangeAttribute)))
    print("visible range:", range(attr(kAXVisibleCharacterRangeAttribute)))
    print("insertion line:", attr(kAXInsertionPointLineNumberAttribute) as? Int ?? -1)
    // A line's range, through the parameterized attribute VoiceOver reads by.
    var line: CFTypeRef?
    if AXUIElementCopyParameterizedAttributeValue(e, kAXRangeForLineParameterizedAttribute as CFString, 2 as CFNumber, &line) == .success {
        print("line 2 range:", range(line))
    }
    var frame: CFTypeRef?
    var r = CFRange(location: 0, length: 3)
    if let rv = AXValueCreate(.cfRange, &r), AXUIElementCopyParameterizedAttributeValue(e, kAXBoundsForRangeParameterizedAttribute as CFString, rv, &frame) == .success, let frame {
        var rect = CGRect.zero; AXValueGetValue(frame as! AXValue, .cgRect, &rect); print("bounds of 0+3:", rect.integral)
    }
case "front":
    // A background process may not take focus for another app (macOS 14 on);
    // System Events may, with Accessibility.
    let p = Process()
    p.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
    p.arguments = ["-e", "tell application \"System Events\" to set frontmost of (first process whose unix id is \(args[2])) to true"]
    try? p.run(); p.waitUntilExit()
    usleep(250000)
default:
    print("unknown: \(args[1])"); exit(2)
}
