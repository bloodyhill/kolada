// The view that shows the note: it asks the model for a frame and puts its
// pixels on screen, and turns touches, the pointer and the keyboard into the
// model's calls. Frames are drawn only when something changed (nothing
// runs while nothing moves).

import UIKit

final class NoteView: UIView, UITextInput, UIDocumentPickerDelegate {
    private(set) var app: OpaquePointer?
    weak var inputDelegate: UITextInputDelegate?
    var markedTextStyle: [NSAttributedString.Key: Any]?
    lazy var tokenizer: UITextInputTokenizer = UITextInputStringTokenizer(textInput: self)
    private var link: CADisplayLink?
    private var due: Timer?
    private var drawQueued = false
    private var momentum: CGFloat = 0
    private var presented = false
    /// Cut, Copy, Paste and Select All by the finger: the system's menu,
    /// over the selection.
    private lazy var editMenu = UIEditMenuInteraction(delegate: nil)
    private var lastTap: (at: CGPoint, time: CFTimeInterval, clicks: UInt32) = (.zero, 0, 0)
    private var saveTimer: Timer?
    /// A trackpad's or mouse's pointer is over the view (iPad).
    private var pointerInside = false
    /// The pan under way began at the left edge (the side menu's swipe).
    private var fromEdge = false
    private var edgeStart = false
    /// What the Files app's picker is open for (`pick`).
    private var picking = ""
    /// The vault's folder, for what iOS needs done to it (iCloud downloads).
    private var vault: URL?

    /// The window's background while the first frame is drawn: graphite or paper.
    var paper: UIColor {
        // notsy-ui theme.rs: graphite 0x111113, paper 0xF7F7F8.
        UIColor { $0.userInterfaceStyle == .dark ? UIColor(red: 0x11 / 255, green: 0x11 / 255, blue: 0x13 / 255, alpha: 1) : UIColor(red: 0xf7 / 255, green: 0xf7 / 255, blue: 0xf8 / 255, alpha: 1) }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        let dark = traitCollection.userInterfaceStyle == .dark
        trace("view made")
        let kept = Vault.kept()
        app = notsy_open(kept?.path ?? "", dark)
        vault = kept ?? FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Notsy")
        trace("model ready")
        notsy_reduce_motion(app, UIAccessibility.isReduceMotionEnabled)
        layer.contentsGravity = .topLeft
        isMultipleTouchEnabled = true
        let tap = UITapGestureRecognizer(target: self, action: #selector(tapped(_:)))
        addGestureRecognizer(tap)
        // A finger (and a trackpad's two) scrolls; a pointer's click-drag selects.
        let pan = UIPanGestureRecognizer(target: self, action: #selector(panned(_:)))
        pan.allowedScrollTypesMask = .all
        pan.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.direct.rawValue), NSNumber(value: UITouch.TouchType.pencil.rawValue)]
        addGestureRecognizer(pan)
        let drag = UIPanGestureRecognizer(target: self, action: #selector(pointerDragged(_:)))
        drag.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.indirectPointer.rawValue)]
        addGestureRecognizer(drag)
        // A long press selects from where it lands, following the finger.
        let press = UILongPressGestureRecognizer(target: self, action: #selector(pressed(_:)))
        addGestureRecognizer(press)
        addInteraction(editMenu)
        addGestureRecognizer(UIHoverGestureRecognizer(target: self, action: #selector(hovered(_:))))
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (view: NoteView, _: UITraitCollection) in
            notsy_set_dark(view.app, view.traitCollection.userInterfaceStyle == .dark)
            view.redraw()
        }
        NotificationCenter.default.addObserver(forName: UIAccessibility.reduceMotionStatusDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
            notsy_reduce_motion(self?.app, UIAccessibility.isReduceMotionEnabled)
        }
    }

    required init?(coder: NSCoder) { fatalError("made in code") }

    deinit { notsy_close(app) }

    /// After the first frame: pictures, the vault's changes, the index.
    func start() {
        let wake: @convention(c) (UnsafeMutableRawPointer?) -> Void = { context in
            guard let context else { return }
            DispatchQueue.main.async {
                let view = Unmanaged<NoteView>.fromOpaque(context).takeUnretainedValue()
                if notsy_poll(view.app) { view.redraw() }
            }
        }
        notsy_start(app, wake, Unmanaged.passUnretained(self).toOpaque())
        becomeFirstResponder()
        fetchFromCloud()
    }

    /// Back in front: what changed in the vault meanwhile (iOS has no
    /// watcher for it: the Files app, iCloud), and what iCloud still owes.
    func cameForward() {
        outside { notsy_do(app, "refresh") }
        fetchFromCloud()
    }

    private func fetchFromCloud() {
        guard let vault else { return }
        Cloud.fetch(vault) { [weak self] in
            guard let self else { return }
            self.outside { notsy_do(self.app, "refresh") }
        }
    }

    func save() {
        saveTimer?.invalidate()
        _ = notsy_do(app, "save")
    }

    /// The note is written a moment after the last edit (0.6 s, as on the
    /// Mac): iOS may end the app at any time, a crash too, so waiting for it
    /// to leave the front would lose what was just typed.
    /// A keystroke only moves the waiting timer on (no timer made per key).
    func edited() {
        if let timer = saveTimer, timer.isValid {
            timer.fireDate = Date(timeIntervalSinceNow: 0.6)
            return
        }
        saveTimer = Timer.scheduledTimer(withTimeInterval: 0.6, repeats: false) { [weak self] _ in
            guard let self else { return }
            _ = notsy_do(self.app, "autosave")
        }
    }

    // MARK: Drawing

    override func layoutSubviews() {
        super.layoutSubviews()
        paint()
    }

    /// Draws again on the next turn of the run loop (many changes, one frame).
    func redraw() {
        if drawQueued { return }
        drawQueued = true
        DispatchQueue.main.async { [weak self] in
            self?.drawQueued = false
            self?.paint()
        }
    }

    private func paint() {
        let scale = window?.screen.scale ?? traitCollection.displayScale
        let w = Int((bounds.width * scale).rounded()), h = Int((bounds.height * scale).rounded())
        guard w > 0, h > 0, let app else { return }
        notsy_frame(app, UInt32(w), UInt32(h), Float(scale))
        if !presented { trace("first frame drawn") }
        let bytes = w * h * 4
        let data = Data(bytes: notsy_pixels(app), count: bytes)
        guard let provider = CGDataProvider(data: data as CFData) else { return }
        let info = CGBitmapInfo(rawValue: CGBitmapInfo.byteOrder32Little.rawValue | CGImageAlphaInfo.noneSkipFirst.rawValue)
        let image = CGImage(width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: info, provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        layer.contents = image
        layer.contentsScale = scale
        CATransaction.commit()
        if !presented {
            presented = true
            trace("first frame presented")
        }
        follow()
    }

    /// Frames at the display's rate while something moves; one step when
    /// something is next due; nothing at rest.
    private func follow() {
        if notsy_wants_frames(app) || momentum != 0 {
            if link == nil {
                link = CADisplayLink(target: self, selector: #selector(step))
                link?.add(to: .main, forMode: .common)
            }
            return
        }
        link?.invalidate()
        link = nil
        due?.invalidate()
        let wait = notsy_next_due(app)
        if wait >= 0 {
            due = Timer.scheduledTimer(withTimeInterval: max(wait, 0.001), repeats: false) { [weak self] _ in
                guard let self else { return }
                _ = notsy_tick(self.app)
                self.paint()
            }
        }
    }

    @objc private func step() {
        if momentum != 0 {
            _ = notsy_scroll(app, Float(momentum))
            momentum *= 0.95
            if abs(momentum) < 0.5 { momentum = 0 }
        }
        _ = notsy_tick(app)
        paint()
    }

    // MARK: Touch and pointer

    /// A tap places the caret at once; taps in quick succession at one
    /// place count up, as clicks do (two: the word, three: the paragraph),
    /// with no wait to see whether another follows.
    @objc private func tapped(_ g: UITapGestureRecognizer) {
        let p = g.location(in: self)
        momentum = 0
        let now = CACurrentMediaTime()
        let near = hypot(p.x - lastTap.at.x, p.y - lastTap.at.y) < 12
        let clicks: UInt32 = near && now - lastTap.time < 0.35 ? min(lastTap.clicks + 1, 3) : 1
        lastTap = (p, now, clicks)
        outside {
            // A finger lifted leaves no pointer behind (a trackpad's does):
            // the model's tap lets a revealed side menu go when it is done with.
            if !pointerInside { return notsy_tap(app, Float(p.x), Float(p.y), clicks) }
            _ = notsy_pointer_down(app, Float(p.x), Float(p.y), clicks, false)
            _ = notsy_pointer_up(app, Float(p.x), Float(p.y))
            return true
        }
        if clicks > 1 { showEditMenu(at: p) }
        if !isFirstResponder { becomeFirstResponder() }
    }

    /// A finger (or two on a trackpad) moves the note; let go, it glides on.
    @objc private func panned(_ g: UIPanGestureRecognizer) {
        switch g.state {
        case .began:
            momentum = 0
            // From the left edge and sideways: the side menu comes out (an
            // iPhone has no pointer to rest near it; its pill takes a tap
            // too). iOS holds an edge touch back a little, so the pan starts
            // some points in, and which way it goes shows at its first move.
            let start = g.location(in: self).x - g.translation(in: self).x
            edgeStart = start < 32
            fromEdge = false
        case .changed where edgeStart:
            let t = g.translation(in: self)
            edgeStart = false
            if t.x > abs(t.y) {
                fromEdge = true
                outside { notsy_do(app, "reveal-menu") }
            }
        case .changed where fromEdge:
            break
        case .changed:
            let dy = g.translation(in: self).y
            g.setTranslation(.zero, in: self)
            _ = notsy_scroll(app, Float(-dy))
            redraw()
        case .ended where fromEdge:
            fromEdge = false
        case .ended:
            momentum = -g.velocity(in: self).y / 60
            redraw()
        default:
            break
        }
    }

    /// Pressed and held: the selection starts there and follows the finger;
    /// let go, the edit menu offers what to do with it.
    @objc private func pressed(_ g: UILongPressGestureRecognizer) {
        let p = g.location(in: self)
        switch g.state {
        case .began:
            momentum = 0
            outside {
                _ = notsy_pointer_down(app, Float(p.x), Float(p.y), 2, false)
                return true
            }
        case .changed:
            outside { notsy_pointer_dragged(app, Float(p.x), Float(p.y)) }
        case .ended:
            outside { notsy_pointer_up(app, Float(p.x), Float(p.y)) }
            showEditMenu(at: p)
        default:
            _ = notsy_pointer_up(app, Float(p.x), Float(p.y))
        }
    }

    /// A trackpad's or a mouse's click-drag: selects, as on the Mac.
    @objc private func pointerDragged(_ g: UIPanGestureRecognizer) {
        let p = g.location(in: self)
        switch g.state {
        case .began:
            let start = CGPoint(x: p.x - g.translation(in: self).x, y: p.y - g.translation(in: self).y)
            outside { notsy_pointer_down(app, Float(start.x), Float(start.y), 1, false) }
            outside { notsy_pointer_dragged(app, Float(p.x), Float(p.y)) }
        case .changed:
            outside { notsy_pointer_dragged(app, Float(p.x), Float(p.y)) }
        default:
            outside { notsy_pointer_up(app, Float(p.x), Float(p.y)) }
        }
    }

    private func showEditMenu(at p: CGPoint) {
        editMenu.presentEditMenu(with: UIEditMenuConfiguration(identifier: nil, sourcePoint: p))
    }

    @objc private func hovered(_ g: UIHoverGestureRecognizer) {
        let p = g.location(in: self)
        switch g.state {
        case .began, .changed:
            pointerInside = true
            if notsy_pointer_moved(app, Float(p.x), Float(p.y)) { redraw() }
        default:
            pointerInside = false
            if notsy_pointer_left(app) { redraw() }
        }
        follow()
    }



    // MARK: Keyboard

    override var canBecomeFirstResponder: Bool { true }

    var hasText: Bool { notsy_length(app) > 0 }

    func insertText(_ text: String) {
        let changed: Bool
        switch text {
        case "\n": changed = notsy_command(app, "insertNewline:")
        case "\t": changed = notsy_command(app, "insertTab:")
        default: changed = notsy_insert(app, text)
        }
        if changed {
            redraw()
            edited()
        }
    }

    func deleteBackward() {
        if notsy_command(app, "deleteBackward:") {
            redraw()
            edited()
        }
    }

    // What the on-screen keyboard leaves alone: Notsy writes what is typed.
    var autocorrectionType: UITextAutocorrectionType { get { .no } set {} }
    var autocapitalizationType: UITextAutocapitalizationType { get { .none } set {} }
    var smartQuotesType: UITextSmartQuotesType { get { .no } set {} }
    var smartDashesType: UITextSmartDashesType { get { .no } set {} }
    var spellCheckingType: UITextSpellCheckingType { get { .no } set {} }

    /// The hardware keyboard: moves by the text system's names, and the
    /// Mac's shortcuts.
    override var keyCommands: [UIKeyCommand]? {
        var commands: [UIKeyCommand] = []
        let moves: [(String, String)] = [(UIKeyCommand.inputLeftArrow, "moveLeft:"), (UIKeyCommand.inputRightArrow, "moveRight:"), (UIKeyCommand.inputUpArrow, "moveUp:"), (UIKeyCommand.inputDownArrow, "moveDown:")]
        let wordMoves = ["moveLeft:": "moveWordLeft:", "moveRight:": "moveWordRight:", "moveUp:": "moveToBeginningOfDocument:", "moveDown:": "moveToEndOfDocument:"]
        let lineMoves = ["moveLeft:": "moveToBeginningOfLine:", "moveRight:": "moveToEndOfLine:", "moveUp:": "moveToBeginningOfDocument:", "moveDown:": "moveToEndOfDocument:"]
        for (input, name) in moves {
            for (flags, base) in [([] as UIKeyModifierFlags, name), (.alternate, wordMoves[name] ?? name), (.command, lineMoves[name] ?? name)] {
                for shift in [false, true] {
                    let full = shift ? base.replacingOccurrences(of: ":", with: "AndModifySelection:") : base
                    let c = UIKeyCommand(action: #selector(textCommand(_:)), input: input, modifierFlags: shift ? flags.union(.shift) : flags, propertyList: full)
                    c.wantsPriorityOverSystemBehavior = true
                    commands.append(c)
                }
            }
        }
        commands.append(UIKeyCommand(action: #selector(textCommand(_:)), input: "\u{8}", modifierFlags: .alternate, propertyList: "deleteWordBackward:"))
        commands.append(UIKeyCommand(action: #selector(appCommand(_:)), input: UIKeyCommand.inputEscape, modifierFlags: [], propertyList: "escape"))
        let shortcuts: [(String, UIKeyModifierFlags, String, String)] = [
            ("n", .command, "new", "New Note"), ("o", .command, "search", "Search Notes"), ("p", .command, "palette", "Commands"),
            ("t", .command, "today", "Today"), ("f", .command, "find", "Find"), ("f", [.command, .alternate], "replace", "Find and Replace"),
            ("h", [.command, .shift], "past", "The Note's Past"), ("z", .command, "undo", "Undo"), ("z", [.command, .shift], "redo", "Redo"),
            ("s", [.command, .control], "menu", "Side Menu"),
        ]
        for (input, flags, what, title) in shortcuts {
            commands.append(UIKeyCommand(title: title, action: #selector(appCommand(_:)), input: input, modifierFlags: flags, propertyList: what))
        }
        commands.append(UIKeyCommand(title: "Select All", action: #selector(textCommand(_:)), input: "a", modifierFlags: .command, propertyList: "selectAll:"))
        return commands
    }

    @objc private func textCommand(_ c: UIKeyCommand) {
        guard let name = c.propertyList as? String else { return }
        outside { notsy_command(app, name) }
    }

    @objc private func appCommand(_ c: UIKeyCommand) {
        guard let what = c.propertyList as? String else { return }
        outside { notsy_do(app, what) }
    }

    /// A change the keyboard did not make (a tap, an arrow key, undo): the
    /// system is told, so its input methods and suggestions stay in step.
    func outside(_ change: () -> Bool) {
        inputDelegate?.selectionWillChange(self)
        inputDelegate?.textWillChange(self)
        let changed = change()
        inputDelegate?.textDidChange(self)
        inputDelegate?.selectionDidChange(self)
        if changed {
            redraw()
            announceChange()
            edited()
        }
        serve()
    }

    /// Does what the model asked of the OS: open a link, fill the
    /// pasteboard, choose a vault.
    func serve() {
        while let e = notsy_take_event(app) {
            let parts = String(cString: e).components(separatedBy: "\t")
            notsy_free_string(e)
            switch parts.first {
            case "url":
                if parts.count > 1, let url = URL(string: parts[1]) { UIApplication.shared.open(url) }
            case "copy":
                UIPasteboard.general.string = parts.dropFirst().joined(separator: "\t")
            case "rich":
                if parts.count > 2 { UIPasteboard.general.items = [["public.utf8-plain-text": parts[1], "public.html": parts[2]]] }
            case "pick":
                if parts.count > 1 { pick(parts[1]) }
            case "paste-new":
                outside { notsy_import_clipboard(app, UIPasteboard.general.string ?? "") }
            case "export":
                if parts.count > 2 { export(parts[1], name: parts[2]) }
            default:
                break
            }
        }
    }

    // MARK: The pasteboard

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        switch action {
        case #selector(copy(_:)), #selector(cut(_:)):
            var location = 0, length = 0
            notsy_selection(app, &location, &length)
            return length > 0
        case #selector(paste(_:)):
            return UIPasteboard.general.hasStrings
        case #selector(selectAll(_:)):
            return true
        default:
            return super.canPerformAction(action, withSender: sender)
        }
    }

    override func copy(_ sender: Any?) {
        guard let s = notsy_selected_text(app) else { return }
        UIPasteboard.general.string = String(cString: s)
        notsy_free_string(s)
    }

    override func cut(_ sender: Any?) {
        copy(sender)
        outside { notsy_insert(app, "") }
    }

    override func paste(_ sender: Any?) {
        let board = UIPasteboard.general
        let plain = board.string ?? ""
        let html = (board.data(forPasteboardType: "public.html")).flatMap { String(data: $0, encoding: .utf8) }
        outside { notsy_paste(app, plain, html) }
    }

    override func selectAll(_ sender: Any?) {
        outside { notsy_command(app, "selectAll:") }
    }

    // MARK: The vault

    /// The Files app's picker: `vault` (Settings, Open another...: a folder
    /// kept through its security-scoped bookmark and opened at once),
    /// `files` to import (copies), `folder` to import.
    func pick(_ what: String) {
        let picker: UIDocumentPickerViewController
        switch what {
        case "files":
            picker = UIDocumentPickerViewController(forOpeningContentTypes: [.item], asCopy: true)
            picker.allowsMultipleSelection = true
        default:
            picker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder])
        }
        picking = what
        picker.delegate = self
        window?.rootViewController?.present(picker, animated: true)
    }

    /// The note sent out through the share sheet (Save to Files, AirDrop,
    /// Mail...) as Markdown, a web page or a PDF printed from it; `choose`
    /// asks which first.
    func export(_ kind: String, name: String) {
        if kind == "choose" {
            let sheet = UIAlertController(title: "Export note", message: nil, preferredStyle: .actionSheet)
            for (title, k) in [("Markdown", "markdown"), ("PDF", "pdf"), ("Web page", "html")] {
                sheet.addAction(UIAlertAction(title: title, style: .default) { [weak self] _ in self?.export(k, name: name) })
            }
            sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            sheet.popoverPresentationController?.sourceView = self
            sheet.popoverPresentationController?.sourceRect = CGRect(x: bounds.maxX - 60, y: 20, width: 1, height: 1)
            window?.rootViewController?.present(sheet, animated: true)
            return
        }
        guard let page = notsy_export(app, kind) else { return }
        let text = String(cString: page)
        notsy_free_string(page)
        let ext = ["markdown": "md", "pdf": "pdf"][kind] ?? "html"
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(name).\(ext)")
        do {
            if kind == "pdf" {
                try pdf(of: text).write(to: url)
            } else {
                try text.write(to: url, atomically: true, encoding: .utf8)
            }
        } catch {
            return
        }
        let share = UIActivityViewController(activityItems: [url], applicationActivities: nil)
        share.popoverPresentationController?.sourceView = self
        share.popoverPresentationController?.sourceRect = CGRect(x: bounds.maxX - 60, y: 20, width: 1, height: 1)
        window?.rootViewController?.present(share, animated: true)
    }

    /// A web page printed to A4 pages (595 x 842 points) with margins.
    private func pdf(of html: String) -> Data {
        let renderer = UIPrintPageRenderer()
        renderer.addPrintFormatter(UIMarkupTextPrintFormatter(markupText: html), startingAtPageAt: 0)
        let paper = CGRect(x: 0, y: 0, width: 595, height: 842)
        renderer.setValue(paper, forKey: "paperRect")
        renderer.setValue(paper.insetBy(dx: 48, dy: 56), forKey: "printableRect")
        let data = NSMutableData()
        UIGraphicsBeginPDFContextToData(data, paper, nil)
        for page in 0..<max(renderer.numberOfPages, 1) {
            UIGraphicsBeginPDFPage()
            renderer.drawPage(at: page, in: UIGraphicsGetPDFContextBounds())
        }
        UIGraphicsEndPDFContext()
        return data as Data
    }
}

/// With NOTSY_TRACE set, says how long since the process began: the same
/// marks the Mac prints, for the launch promise.
func trace(_ what: String) {
    guard ProcessInfo.processInfo.environment["NOTSY_TRACE"] != nil else { return }
    var info = kinfo_proc()
    var size = MemoryLayout<kinfo_proc>.size
    var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
    guard sysctl(&mib, 4, &info, &size, nil, 0) == 0 else { return }
    let start = info.kp_proc.p_un.__p_starttime
    var now = timeval()
    gettimeofday(&now, nil)
    let ms = Double(now.tv_sec - start.tv_sec) * 1000 + Double(now.tv_usec - start.tv_usec) / 1000
    FileHandle.standardError.write("\(what): \(String(format: "%.1f", ms)) ms\n".data(using: .utf8)!)
}

extension NoteView {
    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        switch picking {
        case "files":
            // Copies in the app's inbox: nothing to hold access to.
            outside { notsy_import_files(app, urls.map(\.path).joined(separator: "\n")) }
        case "folder":
            guard let url = urls.first, url.startAccessingSecurityScopedResource() else { return }
            defer { url.stopAccessingSecurityScopedResource() }
            outside { notsy_import_folder(app, url.path) }
        default:
            guard let url = urls.first, let opened = Vault.keep(url) else { return }
            vault = opened
            outside { notsy_switch_vault(app, opened.path) }
            fetchFromCloud()
        }
    }
}

/// The vault folder the user chose, kept across launches as a
/// security-scoped bookmark (a path alone is not enough outside the app's
/// container), with access to it held while the app runs.
enum Vault {
    private static let key = "vault"
    private static var held: URL?

    /// The kept folder, resolved and opened for access; nil before one is
    /// chosen (the app's own Documents/Notsy is used).
    static func kept() -> URL? {
        guard let data = UserDefaults.standard.data(forKey: key) else { return nil }
        var stale = false
        guard let url = try? URL(resolvingBookmarkData: data, bookmarkDataIsStale: &stale) else { return nil }
        guard url.startAccessingSecurityScopedResource() else { return nil }
        held = url
        if stale, let fresh = try? url.bookmarkData() { UserDefaults.standard.set(fresh, forKey: key) }
        return url
    }

    /// Keeps `url` as the vault and holds access to it.
    static func keep(_ url: URL) -> URL? {
        guard url.startAccessingSecurityScopedResource() else { return nil }
        held?.stopAccessingSecurityScopedResource()
        held = url
        if let data = try? url.bookmarkData() { UserDefaults.standard.set(data, forKey: key) }
        return url
    }
}
