// The simulator's hands: Xcode's simulator takes no input from the command
// line, so a simulator build (compiled with KOLADA_DRIVE; never a device
// build) reads steps from the file named by the KOLADA_DRIVE variable and
// does each through the view's own touch, keyboard and pasteboard code,
// saving pictures of the window and a log beside the file.
//
//   tap x y [clicks]   long x y   pan dy   edge   type text   enter   back
//   key cmd+shift+h    move moveWordLeft:   do command   paste text
//   copy   cut   menu x y   pick label   requested   shot name   wait s
//   dark on|off   size w h (the window's width and height, iPad windows)
//   dragin x y a.docx|b.png   dragto x y   dragout   dropit   (files from
//   the app's home, dragged in as another app's drag gives them)
//   open path (a file opened with Kolada)   said (what the page says)
//   notes (how many notes, and the open one's size: never its words)
//   pickvault folder   fontfiles   target Name(0)   clip   signal word
//   tools/ios-drive.sh runs a script in a simulator.

#if KOLADA_DRIVE
import UIKit

enum Driver {
    private static var lines: [String] = []
    private static var menu: [String] = []
    private static var out: URL!

    static func start(_ view: NoteView) {
        guard let path = ProcessInfo.processInfo.environment["KOLADA_DRIVE"], let script = try? String(contentsOfFile: path, encoding: .utf8) else { return }
        out = URL(fileURLWithPath: path).deletingLastPathComponent()
        lines = script.split(separator: "\n").map(String.init).filter { !$0.hasPrefix("#") && !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        log("start \(lines.count) steps")
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { next(view) }
    }

    private static func log(_ s: String) {
        let file = out.appendingPathComponent("drive.log")
        let line = Data((s + "\n").utf8)
        if let h = try? FileHandle(forWritingTo: file) {
            h.seekToEndOfFile()
            h.write(line)
            try? h.close()
        } else {
            try? line.write(to: file)
        }
    }

    private static func next(_ view: NoteView) {
        guard !lines.isEmpty else {
            log("done")
            return
        }
        let line = lines.removeFirst()
        var wait = 0.35
        let f = line.split(separator: " ", maxSplits: 1).map(String.init)
        let rest = f.count > 1 ? f[1] : ""
        let n = rest.split(separator: " ").compactMap { Double($0) }
        let app = view.app
        func p(_ i: Int = 0) -> CGPoint { CGPoint(x: n.count > i ? n[i] : 0, y: n.count > i + 1 ? n[i + 1] : 0) }
        switch f[0] {
        case "tap":
            let clicks = n.count > 2 ? Int(n[2]) : 1
            for _ in 0..<clicks { view.tap(at: p(), flags: []) }
        case "long":
            view.press(.began, at: p())
            view.press(.ended, at: p())
        case "pan":
            _ = kolada_scroll(app, Float(n.first ?? 0))
            view.redraw()
        case "edge":
            view.outside { kolada_do(app, "reveal-menu") }
        case "type":
            for ch in rest { view.insertText(String(ch)) }
        case "enter":
            view.insertText("\n")
        case "back":
            view.deleteBackward()
        case "key":
            var flags: UIKeyModifierFlags = []
            var input = ""
            for part in rest.split(separator: "+") {
                switch part {
                case "cmd": flags.insert(.command)
                case "shift": flags.insert(.shift)
                case "alt": flags.insert(.alternate)
                case "ctrl": flags.insert(.control)
                default: input = String(part)
                }
            }
            if let c = Keys.menus.flatMap(\.1).first(where: { $0.input == input && $0.modifierFlags == flags }) {
                view.appCommand(c)
            } else {
                log("no key \(rest)")
            }
        case "move":
            view.outside { kolada_command(app, rest) }
        case "do":
            view.outside { kolada_do(app, rest) }
        case "paste":
            UIPasteboard.general.string = rest
            view.paste(nil)
        case "copy":
            view.copy(nil)
        case "cut":
            view.cut(nil)
        case "menu", "requested":
            let s = f[0] == "menu" ? kolada_menu(app, view.model(p()).x, view.model(p()).y, false) : kolada_menu_requested(app)
            menu = s.map { String(cString: $0).split(separator: "\n").map { String($0.split(separator: "\t").first ?? "") } } ?? []
            if let s { kolada_free_string(s) }
            log("menu: \(menu.joined(separator: " | "))")
            view.redraw()
        case "pick":
            if let i = menu.firstIndex(of: rest) {
                view.outside { kolada_menu_pick(app, i) }
            } else {
                log("no item \(rest)")
            }
        case "shot":
            if let window = view.window {
                let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in window.drawHierarchy(in: window.bounds, afterScreenUpdates: true) }
                try? image.pngData()?.write(to: out.appendingPathComponent("\(rest).png"))
                log("shot \(rest)")
            }
        case "wait":
            wait = n.first ?? 1
        case "rotate":
            let to: UIInterfaceOrientationMask = rest == "left" ? .landscapeLeft : rest == "right" ? .landscapeRight : .portrait
            view.window?.windowScene?.requestGeometryUpdate(.iOS(interfaceOrientations: to)) { log("rotate: \($0)") }
            wait = 1.5
        case "mark":
            // An input method composing (as a Japanese keyboard does): the text so far.
            view.setMarkedText(rest, selectedRange: NSRange(location: (rest as NSString).length, length: 0))
        case "settle":
            // ...and settling on what it became.
            view.insertText(rest)
        case "outside":
            // Another app writes a note into the vault (Split View, the Files app).
            let parts = rest.split(separator: " ", maxSplits: 1).map(String.init)
            if let vault = view.vault, parts.count == 2 {
                try? parts[1].replacingOccurrences(of: "\\n", with: "\n").write(to: vault.appendingPathComponent(parts[0]), atomically: true, encoding: .utf8)
            }
        case "bench":
            // Keystrokes timed: the model's edit and frame (what a key costs Kolada), median.
            let scale = view.window?.screen.scale ?? 2
            let (w, h) = (UInt32(view.bounds.width * scale), UInt32(view.bounds.height * scale))
            var keys: [Double] = []
            for ch in String(repeating: "typing speed ", count: 6) {
                let t = CACurrentMediaTime()
                _ = kolada_insert(app, String(ch))
                kolada_frame(app, w, h, Float(scale * view.zoom))
                keys.append((CACurrentMediaTime() - t) * 1000)
            }
            keys.sort()
            log(String(format: "keystroke median %.3f ms, worst %.3f ms over %d", keys[keys.count / 2], keys.last ?? 0, keys.count))
            view.redraw()
        case "resign":
            view.resignFirstResponder()
        case "dark":
            view.window?.overrideUserInterfaceStyle = rest == "on" ? .dark : .light
        case "dragin":
            // Files dragged in from another app (the simulator takes no real
            // drag): a drop session of the files named (paths from the app's
            // home), through the view's own drop code.
            let parts = rest.split(separator: " ", maxSplits: 2).map(String.init)
            guard parts.count == 3, let x = Double(parts[0]), let y = Double(parts[1]), let interaction = dropInteraction(view) else { log("dragin x y files"); break }
            let files = parts[2].split(separator: "|").map { URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(String($0)) }
            drag = FakeDrop(files, at: CGPoint(x: x, y: y))
            view.dropInteraction(interaction, sessionDidEnter: drag!)
            let proposal = view.dropInteraction(interaction, sessionDidUpdate: drag!)
            log("drag \(proposal.operation == .copy ? "taken" : "refused"): \(card(app))")
        case "dragto":
            guard let drag, let interaction = dropInteraction(view) else { break }
            drag.point = p()
            let proposal = view.dropInteraction(interaction, sessionDidUpdate: drag)
            log("drag \(proposal.operation == .copy ? "taken" : "refused"): \(card(app))")
        case "dragout":
            guard let drag, let interaction = dropInteraction(view) else { break }
            view.dropInteraction(interaction, sessionDidExit: drag)
            log("drag left: \(card(app))")
            self.drag = nil
        case "dropit":
            guard let drag, let interaction = dropInteraction(view) else { break }
            view.dropInteraction(interaction, performDrop: drag)
            log("dropped")
            self.drag = nil
        case "open":
            // A file opened with Kolada (the share sheet, the Files app).
            view.open(URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(rest))
        case "target":
            // A control by the model's name for it (`Share(Create)`), tapped in its middle.
            var r = [Float](repeating: 0, count: 4)
            if kolada_find(app, rest, &r) {
                view.tap(at: CGPoint(x: view.view(r).midX, y: view.view(r).midY), flags: [])
            } else {
                log("not drawn: \(rest)")
            }
        case "fontfiles":
            // Where each system face was found (the runtime's own, never the Mac's).
            for (name, path) in SystemFonts.found.sorted(by: { $0.key < $1.key }) { log("font \(name): \(path.isEmpty ? "not readable" : path)") }
        case "pickvault":
            // A folder chosen in the Files app as the vault (the picker's answer, as it gives it).
            let url = rest.hasPrefix("/") ? URL(fileURLWithPath: rest) : URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(rest)
            view.picking = "vault"
            view.documentPicker(UIDocumentPickerViewController(forOpeningContentTypes: [.folder]), didPickDocumentsAt: [url])
            log("vault: \(view.vault?.lastPathComponent ?? "none")")
        case "signal":
            // A word in the log for whoever runs the script, to act on (send the app away, write a file).
            log("signal \(rest)")
        case "home":
            // Where the app's data goes: its HOME (the Rust side's) and the container.
            log("HOME \(ProcessInfo.processInfo.environment["HOME"] ?? "unset"), container \(NSHomeDirectory())")
        case "controls":
            // What VoiceOver hears of the chrome: each control's name and kind, through the view's own elements.
            kolada_access(app, true)
            view.redraw()
            DispatchQueue.main.async {
                let found = (view.controls?.accessibilityElements as? [UIAccessibilityElement]) ?? []
                log("controls \(found.count): " + found.map { ($0.accessibilityLabel ?? "") + ($0.accessibilityTraits.contains(.selected) ? " (selected)" : "") }.joined(separator: " | "))
            }
        case "activate":
            // A control activated as VoiceOver does (a double tap anywhere).
            let found = (view.controls?.accessibilityElements as? [UIAccessibilityElement]) ?? []
            if let c = found.first(where: { $0.accessibilityLabel == rest }) { _ = c.accessibilityActivate() } else { log("no control \(rest)") }
        case "textsize":
            // The user's text size (Dynamic Type), as Settings sets it.
            view.window?.rootViewController?.traitOverrides.preferredContentSizeCategory = UIContentSizeCategory(rawValue: rest)
            wait = 1
        case "repaint":
            // The whole view drawn again from the frame (against the changed parts drawn alone).
            view.setNeedsDisplay()
        case "zoom":
            log("zoom \(view.zoom)")
        case "clip":
            log("pasteboard: \(UIPasteboard.general.string ?? "")")
        case "said":
            let s = kolada_whispered(app).map { s -> String in defer { kolada_free_string(s) }; return String(cString: s) } ?? ""
            log("said: \(s)")
        case "notes":
            // How many notes the vault holds, and the open one's shape (never its words).
            let found = view.vault.flatMap { FileManager.default.enumerator(at: $0, includingPropertiesForKeys: nil) }?.compactMap { $0 as? URL }.filter { $0.pathExtension == "md" }.count ?? 0
            let text = kolada_substring(app, 0, kolada_length(app)).map { s -> String in defer { kolada_free_string(s) }; return String(cString: s) } ?? ""
            log("notes \(found) in \(view.vault?.lastPathComponent ?? "no vault"), open note \(text.count) characters, \(text.split(separator: "\n").count) lines")
        case "log":
            var location = 0, length = 0
            kolada_selection(app, &location, &length)
            let text = kolada_substring(app, 0, kolada_length(app)).map { s -> String in defer { kolada_free_string(s) }; return String(cString: s) } ?? ""
            log("text \(text.debugDescription) selection \(location) \(length)")
        default:
            log("unknown \(line)")
        }
        view.serve()
        DispatchQueue.main.asyncAfter(deadline: .now() + wait) { next(view) }
    }

    private static var drag: FakeDrop?

    private static func dropInteraction(_ view: NoteView) -> UIDropInteraction? {
        view.interactions.compactMap { $0 as? UIDropInteraction }.first
    }

    /// What the card over the note says while files are dragged in.
    private static func card(_ app: OpaquePointer?) -> String {
        guard let s = kolada_drag_card(app) else { return "no card" }
        defer { kolada_free_string(s) }
        return String(cString: s).replacingOccurrences(of: "\t", with: " / ")
    }
}

/// A drop session of files, as another app's drag gives them: each an
/// item provider over the file, named, with its type from its extension.
final class FakeDrop: NSObject, UIDropSession {
    let items: [UIDragItem]
    var point: CGPoint
    var progressIndicatorStyle: UIDropSessionProgressIndicatorStyle = .none
    let progress = Progress()

    init(_ files: [URL], at point: CGPoint) {
        items = files.compactMap { url in
            guard let provider = NSItemProvider(contentsOf: url) else { return nil }
            provider.suggestedName = url.deletingPathExtension().lastPathComponent
            return UIDragItem(itemProvider: provider)
        }
        self.point = point
    }

    func location(in view: UIView) -> CGPoint { point }
    var allowsMoveOperation: Bool { false }
    var isRestrictedToDraggingApplication: Bool { false }
    var localDragSession: UIDragSession? { nil }

    func hasItemsConforming(toTypeIdentifiers typeIdentifiers: [String]) -> Bool {
        items.contains { item in typeIdentifiers.contains { item.itemProvider.hasItemConformingToTypeIdentifier($0) } }
    }

    func canLoadObjects(ofClass aClass: NSItemProviderReading.Type) -> Bool { false }

    func loadObjects(ofClass aClass: NSItemProviderReading.Type, completion: @escaping ([NSItemProviderReading]) -> Void) -> Progress {
        completion([])
        return Progress()
    }
}
#endif
