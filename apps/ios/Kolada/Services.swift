// What iOS answers the model with: the table of crates/kolada-ios's
// services.rs (it says what each one does), given once before the first
// window. Spelling from UITextChecker, the web from URLSession, pictures
// from ImageIO, the words in them from Vision, the user's clock, and the
// system's fonts found by name through CoreText (never a scan).

import CoreText
import ImageIO
import UIKit
import UniformTypeIdentifiers
import Vision

/// Hands bytes back to Rust through its sink. (At file scope: a C function
/// pointer may not capture even the type it is made in.)
private func give(_ data: Data, _ sink: KoladaSink?, _ to: UnsafeMutableRawPointer?) {
    guard let sink, !data.isEmpty else { return }
    data.withUnsafeBytes { sink(to, $0.baseAddress, $0.count) }
}

private func give(_ text: String, _ sink: KoladaSink?, _ to: UnsafeMutableRawPointer?) {
    give(Data(text.utf8), sink, to)
}

enum Services {
    /// Gives the model its services; call before the first `kolada_open`.
    static func start() {
        var s = KoladaServices()
        s.spell_check = { units, length, out, most in
            guard let units, let out else { return 0 }
            return Spelling.check(String(utf16CodeUnits: units, count: length), into: out, most: most)
        }
        s.spell_learn = { word in
            guard let word else { return }
            UITextChecker.learnWord(String(cString: word))
        }
        s.spell_guesses = { word, sink, to in
            guard let word else { return }
            give(Spelling.guesses(String(cString: word)).joined(separator: "\n"), sink, to)
        }
        s.web = { method, url, headers, body, bodyLength, most, sink, to in
            guard let method, let url, let address = URL(string: Services.redirected(String(cString: url))) else { return 0 }
            var request = URLRequest(url: address, timeoutInterval: 30)
            request.httpMethod = String(cString: method)
            for line in (headers.map { String(cString: $0) } ?? "").split(separator: "\n") {
                guard let colon = line.firstIndex(of: ":") else { continue }
                request.setValue(line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces), forHTTPHeaderField: String(line[..<colon]))
            }
            if let body, bodyLength > 0 { request.httpBody = Data(bytes: body, count: bodyLength) }
            let answer = Answer()
            URLSession.shared.dataTask(with: request) { data, response, _ in
                answer.status = (response as? HTTPURLResponse)?.statusCode ?? 0
                answer.data = data ?? Data()
                answer.done.signal()
            }.resume()
            answer.done.wait()
            give(answer.data.prefix(most), sink, to)
            return UInt16(clamping: answer.status)
        }
        s.picture_size = { path, bytes, length, width, height in
            guard let source = Pictures.source(path, bytes, length),
                  let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [String: Any],
                  let w = props[kCGImagePropertyPixelWidth as String] as? Int, let h = props[kCGImagePropertyPixelHeight as String] as? Int, w > 0, h > 0
            else { return false }
            width?.pointee = UInt32(w)
            height?.pointee = UInt32(h)
            return true
        }
        s.picture_draw = { path, bytes, length, width, height, bgra in
            guard let bgra, let source = Pictures.source(path, bytes, length), let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
                  let context = Pictures.context(bgra, Int(width), Int(height))
            else { return false }
            context.interpolationQuality = .high
            context.draw(image, in: CGRect(x: 0, y: 0, width: Int(width), height: Int(height)))
            return true
        }
        s.picture_encode = { bgra, width, height, quality, png, sink, to in
            guard let bgra, let context = Pictures.context(UnsafeMutablePointer(mutating: bgra), Int(width), Int(height)), let image = context.makeImage() else { return false }
            let out = NSMutableData()
            let kind = (png ? UTType.png : UTType.jpeg).identifier as CFString
            guard let destination = CGImageDestinationCreateWithData(out, kind, 1, nil) else { return false }
            CGImageDestinationAddImage(destination, image, [kCGImageDestinationLossyCompressionQuality: Double(quality) / 100] as CFDictionary)
            guard CGImageDestinationFinalize(destination) else { return false }
            give(out as Data, sink, to)
            return true
        }
        s.picture_text = { path, sink, to in
            guard let path else { return false }
            let request = VNRecognizeTextRequest()
            request.recognitionLevel = .accurate
            request.automaticallyDetectsLanguage = true
            let handler = VNImageRequestHandler(url: URL(fileURLWithPath: String(cString: path)))
            // A file Vision cannot read holds no words: it is not asked again.
            guard (try? handler.perform([request])) != nil else { return true }
            let lines = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
            give(lines.joined(separator: "\n"), sink, to)
            return true
        }
        s.clock = { seconds, sink, to in
            give(Clock.formatter.string(from: Date(timeIntervalSince1970: TimeInterval(seconds))), sink, to)
        }
        s.font_file = { name, sink, to in
            guard let name else { return }
            give(SystemFonts.file(String(cString: name)), sink, to)
        }
        s.color_glyph = { name, glyph, size, left, top, width, height, sink, to in
            guard let name else { return false }
            // iOS keeps its emoji in a form only CoreText draws. The face
            // the engine took is the collection's UI one (".AppleColorEmojiUI"),
            // which CoreText gives out under its public name, the same glyphs.
            var ps = String(cString: name)
            if ps.hasPrefix(".") { ps = String(ps.dropFirst()) }
            if ps.hasSuffix("UI") { ps = String(ps.dropLast(2)) }
            let font = CTFontCreateWithName(ps as CFString, CGFloat(size), nil)
            var g = CGGlyph(glyph)
            let box = CTFontGetBoundingRectsForGlyphs(font, .horizontal, &g, nil, 1)
            guard !box.isNull, box.width > 0, box.height > 0 else { return false }
            let (x0, y0, x1, y1) = (box.minX.rounded(.down), box.minY.rounded(.down), box.maxX.rounded(.up), box.maxY.rounded(.up))
            let (w, h) = (Int(x1 - x0), Int(y1 - y0))
            var data = Data(count: w * h * 4)
            let drawn = data.withUnsafeMutableBytes { raw -> Bool in
                guard let context = CGContext(data: raw.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
                var at = CGPoint(x: -x0, y: -y0)
                CTFontDrawGlyphs(font, &g, &at, 1, context)
                return true
            }
            guard drawn else { return false }
            left?.pointee = Int32(x0)
            top?.pointee = Int32(y1)
            width?.pointee = UInt32(w)
            height?.pointee = UInt32(h)
            give(data, sink, to)
            return true
        }
        s.recycle = { path in
            guard let path else { return false }
            // The Files app's Recently Deleted, where it can bring it back
            // (a vault in iCloud Drive). The app's own folder has none: there
            // the note, already out of Kolada's trash by the user's word, goes.
            let url = URL(fileURLWithPath: String(cString: path))
            if (try? FileManager.default.trashItem(at: url, resultingItemURL: nil)) != nil { return true }
            return (try? FileManager.default.removeItem(at: url)) != nil
        }
        s.app_installed = { name in
            // The apps whose links Kolada may ask about (LSApplicationQueriesSchemes).
            guard let name, String(cString: name) == "Claude", let url = URL(string: "claude://") else { return false }
            return Thread.isMainThread && UIApplication.shared.canOpenURL(url)
        }
        let fonts = Bundle.main.bundleURL.appendingPathComponent("Fonts").path
        fonts.withCString { dir in
            s.fonts_dir = dir
            kolada_services(&s)
        }
    }

    /// `address`, sent to a local copy of the sharing service instead of
    /// notsy.co when a simulator build is given one (KOLADA_SHARE_BASE, as
    /// `http://127.0.0.1:8787`), so testing share links publishes nothing.
    /// A device build always talks to notsy.co.
    static func redirected(_ address: String) -> String {
        #if KOLADA_DRIVE
        let real = "https://notsy.co"
        if let base = ProcessInfo.processInfo.environment["KOLADA_SHARE_BASE"], address == real || address.hasPrefix(real + "/") {
            return base + address.dropFirst(real.count)
        }
        #endif
        return address
    }
}

/// A request's answer, waited for on the thread that asked.
private final class Answer: @unchecked Sendable {
    var status = 0
    var data = Data()
    let done = DispatchSemaphore(value: 0)
}

private enum Clock {
    /// The user's clock: 12 or 24 hour, as Settings has it.
    static let formatter: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .none
        f.timeStyle = .short
        return f
    }()
}

private enum Spelling {
    static let checker = UITextChecker()

    /// The dictionary for `text`: Arabic for Arabic letters, Hebrew for
    /// Hebrew, else the user's own language (English when it has none).
    static func language(_ text: String) -> String {
        let available = UITextChecker.availableLanguages
        func pick(_ code: String) -> String? { available.first { $0 == code || $0.hasPrefix(code + "_") || $0.hasPrefix(code + "-") } }
        if text.unicodeScalars.contains(where: { (0x0600...0x06FF).contains($0.value) }), let ar = pick("ar") { return ar }
        if text.unicodeScalars.contains(where: { (0x0590...0x05FF).contains($0.value) }), let he = pick("he") { return he }
        for preferred in Locale.preferredLanguages {
            if available.contains(preferred) { return preferred }
            let code = String(preferred.prefix { $0 != "-" && $0 != "_" })
            if code != "ar", code != "he", let found = pick(code) { return found }
        }
        return pick("en") ?? available.first ?? "en_US"
    }

    static func check(_ text: String, into out: UnsafeMutablePointer<Int>, most: Int) -> Int {
        let ns = text as NSString
        let lang = language(text)
        var at = 0, found = 0
        while found < most && at < ns.length {
            // Never wrapping round to the line's start: that went on for ever on the Mac.
            let r = checker.rangeOfMisspelledWord(in: text, range: NSRange(location: 0, length: ns.length), startingAt: at, wrap: false, language: lang)
            guard r.location != NSNotFound, r.length > 0 else { break }
            out[found * 2] = r.location
            out[found * 2 + 1] = r.location + r.length
            found += 1
            at = r.location + r.length
        }
        return found
    }

    static func guesses(_ word: String) -> [String] {
        Array((checker.guesses(forWordRange: NSRange(location: 0, length: (word as NSString).length), in: word, language: language(word)) ?? []).prefix(5))
    }
}

private enum Pictures {
    static func source(_ path: UnsafePointer<CChar>?, _ bytes: UnsafePointer<UInt8>?, _ length: Int) -> CGImageSource? {
        if let path { return CGImageSourceCreateWithURL(URL(fileURLWithPath: String(cString: path)) as CFURL, nil) }
        guard let bytes, length > 0 else { return nil }
        return CGImageSourceCreateWithData(Data(bytes: bytes, count: length) as CFData, nil)
    }

    /// A bitmap over `pixels`: little-endian 0xAARRGGBB words, premultiplied, top row first.
    static func context(_ pixels: UnsafeMutablePointer<UInt8>, _ width: Int, _ height: Int) -> CGContext? {
        guard width > 0, height > 0 else { return nil }
        let info = CGBitmapInfo.byteOrder32Little.rawValue | CGImageAlphaInfo.premultipliedFirst.rawValue
        return CGContext(data: pixels, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: info)
    }
}

/// The system's fonts by PostScript name, as CoreText finds them on this
/// device (or this simulator's runtime), kept per iOS version so later
/// launches only name files.
enum SystemFonts {
    private static let key = "fonts " + ProcessInfo.processInfo.operatingSystemVersionString
    private static var known: [String: String] = UserDefaults.standard.dictionary(forKey: key) as? [String: String] ?? [:]

    /// The faces found so far, by PostScript name (empty: not readable here).
    static var found: [String: String] { known }

    static func file(_ name: String) -> String {
        #if KOLADA_DRIVE
        // Simulator builds can be made to draw as a device whose system fonts cannot be read.
        if ProcessInfo.processInfo.environment["KOLADA_FONTS"] == "bundled" { return "" }
        #endif
        // Kept, and still there (the simulator's runtime moves after a restart).
        if let path = known[name], path.isEmpty || FileManager.default.fileExists(atPath: path) { return path }
        var path = ""
        let font = CTFontCreateWithName(name as CFString, 12, nil)
        // CoreText stands another font in for a name it lacks: only the one asked for counts.
        if (CTFontCopyPostScriptName(font) as String).caseInsensitiveCompare(name) == .orderedSame,
           let url = CTFontCopyAttribute(font, kCTFontURLAttribute) as? URL,
           FileManager.default.isReadableFile(atPath: url.path) {
            path = url.path
        }
        known[name] = path
        UserDefaults.standard.set(known, forKey: key)
        return path
    }
}
