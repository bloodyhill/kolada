// An empty AppKit window, with the same launch marks as kolada-mac prints under
// KOLADA_TRACE: the floor any Mac app pays for AppKit itself. Kolada's launch is
// judged against it (docs/porting.md). Build and run it on the Mac:
//
//   mkdir -p /tmp/Base.app/Contents/MacOS && swiftc -O tools/mac-launch-baseline.swift -o /tmp/Base.app/Contents/MacOS/Base
//   (an Info.plist with CFBundleExecutable Base, then) open -n /tmp/Base.app --stderr /tmp/base.err

import AppKit
func age() -> Double {
    var info = proc_bsdinfo()
    proc_pidinfo(getpid(), PROC_PIDTBSDINFO, 0, &info, Int32(MemoryLayout<proc_bsdinfo>.size))
    let now = Date().timeIntervalSince1970
    return (now - Double(info.pbi_start_tvsec) - Double(info.pbi_start_tvusec) / 1e6) * 1000
}
func mark(_ s: String) { FileHandle.standardError.write("\(s): \(String(format: "%.1f", age())) ms\n".data(using: .utf8)!) }
mark("main")
let app = NSApplication.shared
mark("application created")
app.setActivationPolicy(.regular)
class Delegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    func applicationDidFinishLaunching(_ n: Notification) { mark("launched") }
}
class V: NSView { var first = true
    override func draw(_ r: NSRect) {
        NSColor.black.setFill(); r.fill()
        if first {
            first = false; mark("first draw")
            // The speed gate (tools/speed-gate-mac.sh) runs it to its first draw.
            if ProcessInfo.processInfo.environment["KOLADA_EXIT_AFTER_FIRST_FRAME"] != nil {
                DispatchQueue.main.async { NSApp.terminate(nil) }
            }
        }
    } }
let d = Delegate(); app.delegate = d
let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1000, height: 720), styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
w.contentView = V(); w.center(); w.makeKeyAndOrderFront(nil); d.window = w
mark("window shown")
app.run()
