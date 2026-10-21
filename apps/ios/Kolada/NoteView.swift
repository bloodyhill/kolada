// The view that shows the note: it asks the model for a frame and puts its
// pixels on screen, redrawing only where they changed, and turns touches,
// the pointer and the keyboard into the model's calls. Frames are drawn
// only when something changed (nothing runs while nothing moves).

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
    /// The last frame, over the model's own pixels (no copy): `draw(_:)`
    /// puts the parts that changed on screen.
    private var frameImage: CGImage?
    /// The edit menu: the model's menus by the finger (the note's, a row's).
    lazy var editMenu = UIEditMenuInteraction(delegate: self)
    private var lastTap: (at: CGPoint, time: CFTimeInterval, clicks: UInt32) = (.zero, 0, 0)
    private var saveTimer: Timer?
    private var spellTimer: Timer?
    /// A trackpad's or mouse's pointer is over the view (iPad).
    private var pointerInside = false
    /// The pan under way began at the left edge (the side menu's swipe).
    private var fromEdge = false
    private var edgeStart = false
    /// What a long press under way is on (`kolada_press_kind`).
    private var pressKind: UInt8 = 0
    /// What the Files app's picker is open for (`pick`).
    var picking = ""
    /// The vault's folder, for what iOS needs done to it (iCloud downloads).
    var vault: URL?
    /// The chrome and the note drawn larger for the user's text size
    /// (Dynamic Type): a model point is `zoom` view points. 1 at the
    /// default size.
    private(set) var zoom: CGFloat = 1
    /// VoiceOver's names for the chrome's controls (Access.swift).
    weak var controls: ControlsView?

    /// A view point as the model counts it.
    func model(_ p: CGPoint) -> (x: Float, y: Float) { (Float(p.x / zoom), Float(p.y / zoom)) }

    /// A model box (left, top, right, bottom) as view points.
    func view(_ r: [Float]) -> CGRect {
        CGRect(x: CGFloat(r[0]) * zoom, y: CGFloat(r[1]) * zoom, width: CGFloat(max(r[2] - r[0], 1)) * zoom, height: CGFloat(r[3] - r[1]) * zoom)
    }

    /// The zoom for the user's text size: larger past the default, never
    /// smaller, and never so large the window is narrower than 320 model
    /// points (the narrowest the chrome is laid out for).
    func followTextSize() {
        let steps: [UIContentSizeCategory: CGFloat] = [
            .extraLarge: 1.1, .extraExtraLarge: 1.2, .extraExtraExtraLarge: 1.3,
            .accessibilityMedium: 1.4, .accessibilityLarge: 1.5, .accessibilityExtraLarge: 1.6,
            .accessibilityExtraExtraLarge: 1.7, .accessibilityExtraExtraExtraLarge: 1.8,
        ]
        let wanted = steps[traitCollection.preferredContentSizeCategory] ?? 1
        let fits = bounds.width > 0 ? max(1, bounds.width / 320) : wanted
        let z = (min(wanted, fits) * 20).rounded(.down) / 20
        if z != zoom {
            zoom = z
            setNeedsLayout()
        }
    }

    /// The window's background while the first frame is drawn: graphite or paper.
    var paper: UIColor {
        // kolada-ui theme.rs: graphite 0x1A1A1A, paper 0xF7F7F8.
        UIColor { $0.userInterfaceStyle == .dark ? UIColor(red: 0x1a / 255, green: 0x1a / 255, blue: 0x1a / 255, alpha: 1) : UIColor(red: 0xf7 / 255, green: 0xf7 / 255, blue: 0xf8 / 255, alpha: 1) }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        let dark = traitCollection.userInterfaceStyle == .dark
        trace("view made")
        let kept = Vault.kept()
        Services.start()
        app = kolada_open(kept?.path ?? "", dark)
        vault = kept ?? FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Kolada")
        trace("model ready")
        kolada_reduce_motion(app, UIAccessibility.isReduceMotionEnabled)
        isOpaque = true
        backgroundColor = paper
        contentMode = .redraw
        isMultipleTouchEnabled = true
        let tap = UITapGestureRecognizer(target: self, action: #selector(tapped(_:)))
        addGestureRecognizer(tap)
        // A trackpad's or mouse's secondary click: the menu for what is there.
        let secondary = UITapGestureRecognizer(target: self, action: #selector(secondaryClicked(_:)))
        secondary.buttonMaskRequired = .secondary
        addGestureRecognizer(secondary)
        // A finger (and a trackpad's two) scrolls; a pointer's click-drag selects.
        let pan = UIPanGestureRecognizer(target: self, action: #selector(panned(_:)))
        pan.allowedScrollTypesMask = .all
        pan.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.direct.rawValue), NSNumber(value: UITouch.TouchType.pencil.rawValue)]
        addGestureRecognizer(pan)
        let drag = UIPanGestureRecognizer(target: self, action: #selector(pointerDragged(_:)))
        drag.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.indirectPointer.rawValue)]
        addGestureRecognizer(drag)
        // A long press selects from where it lands, following the finger;
        // on a row of the side menu it opens the row's menu.
        let press = UILongPressGestureRecognizer(target: self, action: #selector(pressed(_:)))
        addGestureRecognizer(press)
        addInteraction(editMenu)
        addInteraction(UIDropInteraction(delegate: self))
        addGestureRecognizer(UIHoverGestureRecognizer(target: self, action: #selector(hovered(_:))))
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (view: NoteView, _: UITraitCollection) in
            kolada_set_dark(view.app, view.traitCollection.userInterfaceStyle == .dark)
            view.redraw()
        }
        registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { (view: NoteView, _: UITraitCollection) in
            view.followTextSize()
        }
        NotificationCenter.default.addObserver(forName: UIAccessibility.reduceMotionStatusDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
            kolada_reduce_motion(self?.app, UIAccessibility.isReduceMotionEnabled)
        }
        kolada_access(app, UIAccessibility.isVoiceOverRunning)
        NotificationCenter.default.addObserver(forName: UIAccessibility.voiceOverStatusDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
            guard let self else { return }
            kolada_access(self.app, UIAccessibility.isVoiceOverRunning)
            self.redraw()
        }
    }

    required init?(coder: NSCoder) { fatalError("made in code") }

    deinit { kolada_close(app) }

    /// After the first frame: pictures, the vault's changes, the index.
    func start() {
        let wake: @convention(c) (UnsafeMutableRawPointer?) -> Void = { context in
            guard let context else { return }
            DispatchQueue.main.async {
                let view = Unmanaged<NoteView>.fromOpaque(context).takeUnretainedValue()
                if kolada_poll(view.app) { view.redraw() }
                view.serve()
            }
        }
        kolada_start(app, wake, Unmanaged.passUnretained(self).toOpaque())
        becomeFirstResponder()
        fetchFromCloud()
        checkSpelling(after: 0.05)
        #if KOLADA_DRIVE
        Driver.start(self)
        #endif
    }

    /// Back in front: what changed in the vault meanwhile (iOS has no
    /// watcher for it: the Files app, iCloud), and what iCloud still owes.
    func cameForward() {
        outside { kolada_do(app, "refresh") }
        fetchFromCloud()
    }

    func fetchFromCloud() {
        guard let vault else { return }
        Cloud.fetch(vault) { [weak self] in
            guard let self else { return }
            self.outside { kolada_do(self.app, "refresh") }
        }
    }

    func save() {
        saveTimer?.invalidate()
        _ = kolada_do(app, "save")
    }

    /// The note is written a moment after the last edit (0.6 s, as on the
    /// Mac): iOS may end the app at any time, a crash too, so waiting for it
    /// to leave the front would lose what was just typed.
    /// A keystroke only moves the waiting timers on (none made per key).
    func edited() {
        checkSpelling(after: 0.4)
        if let timer = saveTimer, timer.isValid {
            timer.fireDate = Date(timeIntervalSinceNow: 0.6)
            return
        }
        saveTimer = Timer.scheduledTimer(withTimeInterval: 0.6, repeats: false) { [weak self] _ in
            guard let self else { return }
            _ = kolada_do(self.app, "autosave")
        }
    }

    /// Spelling is checked a few lines at a time once typing pauses.
    private func checkSpelling(after delay: TimeInterval) {
        if let timer = spellTimer, timer.isValid {
            timer.fireDate = Date(timeIntervalSinceNow: delay)
            return
        }
        spellTimer = Timer.scheduledTimer(withTimeInterval: delay, repeats: false) { [weak self] _ in
            guard let self else { return }
            var more = false
            if kolada_spell_step(self.app, &more) { self.redraw() }
            if more { self.checkSpelling(after: 0.02) }
        }
    }

    // MARK: Drawing

    override func layoutSubviews() {
        super.layoutSubviews()
        followTextSize()
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

    private var damage = [Int32](repeating: 0, count: 64 * 4)

    private func paint() {
        let scale = window?.screen.scale ?? traitCollection.displayScale
        let w = Int((bounds.width * scale).rounded()), h = Int((bounds.height * scale).rounded())
        guard w > 0, h > 0, let app else { return }
        if !presented { trace("first frame asked") }
        kolada_frame(app, UInt32(w), UInt32(h), Float(scale * zoom))
        if !presented { trace("first frame drawn") }
        // The model's pixels as they are, no copy: drawn before it draws again.
        let bytes = w * h * 4
        guard let pixels = kolada_pixels(app),
              let provider = CGDataProvider(dataInfo: nil, data: UnsafeRawPointer(pixels), size: bytes, releaseData: { _, _, _ in })
        else { return }
        let info = CGBitmapInfo(rawValue: CGBitmapInfo.byteOrder32Little.rawValue | CGImageAlphaInfo.noneSkipFirst.rawValue)
        frameImage = CGImage(width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: info, provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
        let n = kolada_damage(app, &damage, 64)
        if n < 0 || !presented {
            setNeedsDisplay()
        } else {
            for i in 0..<Int(n) {
                let r = damage[(i * 4)..<(i * 4 + 4)].map { CGFloat($0) / scale }
                setNeedsDisplay(CGRect(x: r[0], y: r[1], width: r[2] - r[0], height: r[3] - r[1]))
            }
        }
        if !presented {
            presented = true
            trace("first frame presented")
        }
        controls?.drawn()
        follow()
    }

    override func draw(_ rect: CGRect) {
        guard let image = frameImage, let context = UIGraphicsGetCurrentContext() else { return }
        let scale = contentScaleFactor
        context.interpolationQuality = .none
        context.saveGState()
        // UIKit's context is flipped; Core Graphics draws images the other way up.
        context.translateBy(x: 0, y: CGFloat(image.height) / scale)
        context.scaleBy(x: 1, y: -1)
        context.draw(image, in: CGRect(x: 0, y: 0, width: CGFloat(image.width) / scale, height: CGFloat(image.height) / scale))
        context.restoreGState()
    }

    /// Frames at the display's rate while something moves; one step when
    /// something is next due; nothing at rest.
    private func follow() {
        if kolada_wants_frames(app) || momentum != 0 {
            if link == nil {
                link = CADisplayLink(target: self, selector: #selector(step))
                link?.add(to: .main, forMode: .common)
            }
            return
        }
        link?.invalidate()
        link = nil
        due?.invalidate()
        let wait = kolada_next_due(app)
        if wait >= 0 {
            due = Timer.scheduledTimer(withTimeInterval: max(wait, 0.001), repeats: false) { [weak self] _ in
                guard let self else { return }
                _ = kolada_tick(self.app)
                self.paint()
            }
        }
    }

    @objc private func step() {
        if momentum != 0 {
            _ = kolada_scroll(app, Float(momentum / zoom))
            momentum *= 0.95
            if abs(momentum) < 0.5 { momentum = 0 }
        }
        _ = kolada_tick(app)
        paint()
    }

    // MARK: Touch and pointer

    /// A tap places the caret at once; taps in quick succession at one
    /// place count up, as clicks do (two: the word, three: the paragraph),
    /// with no wait to see whether another follows. With Command held (a
    /// hardware keyboard), a link is followed, as a Command-click on the Mac.
    @objc private func tapped(_ g: UITapGestureRecognizer) {
        tap(at: g.location(in: self), flags: g.modifierFlags)
    }

    func tap(at p: CGPoint, flags: UIKeyModifierFlags) {
        momentum = 0
        if flags.contains(.command), kolada_link_click(app, model(p).x, model(p).y) {
            redraw()
            serve()
            return
        }
        let now = CACurrentMediaTime()
        let near = hypot(p.x - lastTap.at.x, p.y - lastTap.at.y) < 12
        let clicks: UInt32 = near && now - lastTap.time < 0.35 ? min(lastTap.clicks + 1, 3) : 1
        lastTap = (p, now, clicks)
        outside {
            // A finger lifted leaves no pointer behind (a trackpad's does):
            // the model's tap lets a revealed side menu go when it is done with.
            if !pointerInside { return kolada_tap(app, model(p).x, model(p).y, clicks) }
            _ = kolada_pointer_down(app, model(p).x, model(p).y, clicks, flags.contains(.shift))
            _ = kolada_pointer_up(app, model(p).x, model(p).y)
            return true
        }
        if clicks > 1 { showMenu(at: p) }
        if !isFirstResponder { becomeFirstResponder() }
    }

    @objc private func secondaryClicked(_ g: UITapGestureRecognizer) {
        showMenu(at: g.location(in: self))
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
                outside { kolada_do(app, "reveal-menu") }
            }
        case .changed where fromEdge:
            break
        case .changed:
            let dy = g.translation(in: self).y
            g.setTranslation(.zero, in: self)
            _ = kolada_scroll(app, Float(-dy / zoom))
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

    /// Pressed and held on the text: the selection starts there and follows
    /// the finger; let go, the menu offers what to do with it. On a row of
    /// the side menu: the row's menu, at once.
    @objc private func pressed(_ g: UILongPressGestureRecognizer) {
        press(g.state, at: g.location(in: self))
    }

    func press(_ state: UIGestureRecognizer.State, at p: CGPoint) {
        switch state {
        case .began:
            momentum = 0
            pressKind = kolada_press_kind(app, model(p).x, model(p).y)
            if pressKind == 2 {
                showMenu(at: p)
                return
            }
            outside {
                _ = kolada_pointer_down(app, model(p).x, model(p).y, 2, false)
                return true
            }
        case .changed where pressKind != 2:
            outside { kolada_pointer_dragged(app, model(p).x, model(p).y) }
        case .ended where pressKind != 2:
            outside { kolada_pointer_up(app, model(p).x, model(p).y) }
            showMenu(at: p)
        case .cancelled, .failed:
            if pressKind != 2 { _ = kolada_pointer_up(app, model(p).x, model(p).y) }
        default:
            break
        }
    }

    /// A trackpad's or a mouse's click-drag: selects, as on the Mac.
    @objc private func pointerDragged(_ g: UIPanGestureRecognizer) {
        let p = g.location(in: self)
        switch g.state {
        case .began:
            let start = CGPoint(x: p.x - g.translation(in: self).x, y: p.y - g.translation(in: self).y)
            outside { kolada_pointer_down(app, model(start).x, model(start).y, 1, false) }
            outside { kolada_pointer_dragged(app, model(p).x, model(p).y) }
        case .changed:
            outside { kolada_pointer_dragged(app, model(p).x, model(p).y) }
        default:
            outside { kolada_pointer_up(app, model(p).x, model(p).y) }
        }
    }

    @objc private func hovered(_ g: UIHoverGestureRecognizer) {
        let p = g.location(in: self)
        switch g.state {
        case .began, .changed:
            pointerInside = true
            if kolada_pointer_moved(app, model(p).x, model(p).y) { redraw() }
        default:
            pointerInside = false
            if kolada_pointer_left(app) { redraw() }
        }
        follow()
    }

    // MARK: Keyboard

    override var canBecomeFirstResponder: Bool { true }

    var hasText: Bool { kolada_length(app) > 0 }

    func insertText(_ text: String) {
        let changed: Bool
        switch text {
        case "\n": changed = kolada_command(app, "insertNewline:")
        case "\t": changed = kolada_command(app, "insertTab:")
        default: changed = kolada_insert(app, text)
        }
        if changed {
            redraw()
            edited()
        }
    }

    func deleteBackward() {
        if kolada_command(app, "deleteBackward:") {
            redraw()
            edited()
        }
    }

    // What the on-screen keyboard leaves alone: Kolada writes what is typed,
    // and marks spelling itself (UITextChecker, through the model).
    var autocorrectionType: UITextAutocorrectionType { get { .no } set {} }
    var autocapitalizationType: UITextAutocapitalizationType { get { .none } set {} }
    var smartQuotesType: UITextSmartQuotesType { get { .no } set {} }
    var smartDashesType: UITextSmartDashesType { get { .no } set {} }
    var spellCheckingType: UITextSpellCheckingType { get { .no } set {} }

    /// The hardware keyboard's moves, by the text system's names (the app's
    /// commands are in the menu bar: Keys.swift).
    override var keyCommands: [UIKeyCommand]? { Keys.moves }

    @objc func textCommand(_ c: UIKeyCommand) {
        guard let name = c.propertyList as? String else { return }
        outside { kolada_command(app, name) }
    }

    /// A command of the menu bar or a shortcut (`kolada_commands`).
    @objc func appCommand(_ c: UICommand) {
        guard let what = c.propertyList as? String else { return }
        if what == "paste-plain" { return pastePlain(c) }
        outside { kolada_do(app, what) }
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
    /// pasteboard, choose a vault, show a menu.
    func serve() {
        while let e = kolada_take_event(app) {
            let parts = String(cString: e).components(separatedBy: "\t")
            kolada_free_string(e)
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
                outside { kolada_import_clipboard(app, UIPasteboard.general.string ?? "") }
            case "export":
                if parts.count > 2 { export(parts[1], name: parts[2]) }
            case "menu":
                if parts.count > 2, let x = Double(parts[1]), let y = Double(parts[2]) { showRequestedMenu(at: CGPoint(x: CGFloat(x) * zoom, y: CGFloat(y) * zoom)) }
            case "edit":
                switch parts.count > 1 ? parts[1] : "" {
                case "cut": cut(nil)
                case "copy": copy(nil)
                case "paste": paste(nil)
                case "paste-plain": pastePlain(nil)
                default: break
                }
            default:
                break
            }
        }
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

    /// A file opened with Kolada (the share sheet, the Files app): brought
    /// in, or opened when it is one of the vault's own.
    func open(_ url: URL) {
        let held = url.startAccessingSecurityScopedResource()
        defer { if held { url.stopAccessingSecurityScopedResource() } }
        outside { kolada_open_file(app, url.path) }
        // The copy iOS left in the app's inbox is done with.
        if url.path.contains("/Documents/Inbox/") { try? FileManager.default.removeItem(at: url) }
    }
}

/// With KOLADA_TRACE set, says how long since the process began: the same
/// marks the Mac prints, for the launch promise.
func trace(_ what: String) {
    guard ProcessInfo.processInfo.environment["KOLADA_TRACE"] != nil else { return }
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
            outside { kolada_import_files(app, urls.map(\.path).joined(separator: "\n")) }
        case "folder":
            guard let url = urls.first, url.startAccessingSecurityScopedResource() else { return }
            defer { url.stopAccessingSecurityScopedResource() }
            outside { kolada_import_folder(app, url.path) }
        default:
            guard let url = urls.first, let opened = Vault.keep(url) else { return }
            vault = opened
            outside { kolada_switch_vault(app, opened.path) }
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
    /// chosen (the app's own Documents/Kolada is used).
    static func kept() -> URL? {
        guard let data = UserDefaults.standard.data(forKey: key) else { return nil }
        var stale = false
        guard let url = try? URL(resolvingBookmarkData: data, bookmarkDataIsStale: &stale), hold(url) else { return nil }
        if stale, let fresh = try? url.bookmarkData() { UserDefaults.standard.set(fresh, forKey: key) }
        return url
    }

    /// Keeps `url` as the vault and holds access to it.
    static func keep(_ url: URL) -> URL? {
        let before = held
        guard hold(url) else { return nil }
        if before != url { before?.stopAccessingSecurityScopedResource() }
        if let data = try? url.bookmarkData() { UserDefaults.standard.set(data, forKey: key) }
        return url
    }

    /// Access to `url` held: a folder from the Files app needs its scope
    /// opened; one the app may read anyway (its own container) does not.
    private static func hold(_ url: URL) -> Bool {
        let scoped = url.startAccessingSecurityScopedResource()
        guard scoped || FileManager.default.isReadableFile(atPath: url.path) else { return false }
        held = url
        return true
    }
}
