// The hardware keyboard: the text system's moves on the note, and the app's
// commands in the iPadOS menu bar with the Mac's shortcuts (the table is
// crates/kolada-ios/src/keys.rs, given by kolada_commands). Holding Command
// shows them; each is sent to the note view as `appCommand:`.

import UIKit

enum Keys {
    /// Arrows, alone, with Option (words) and Command (lines, the note's
    /// ends), Shift extending; Option-Delete deletes a word.
    static let moves: [UIKeyCommand] = {
        var commands: [UIKeyCommand] = []
        let moves: [(String, String)] = [(UIKeyCommand.inputLeftArrow, "moveLeft:"), (UIKeyCommand.inputRightArrow, "moveRight:"), (UIKeyCommand.inputUpArrow, "moveUp:"), (UIKeyCommand.inputDownArrow, "moveDown:")]
        let wordMoves = ["moveLeft:": "moveWordLeft:", "moveRight:": "moveWordRight:", "moveUp:": "moveToBeginningOfDocument:", "moveDown:": "moveToEndOfDocument:"]
        let lineMoves = ["moveLeft:": "moveToBeginningOfLine:", "moveRight:": "moveToEndOfLine:", "moveUp:": "moveToBeginningOfDocument:", "moveDown:": "moveToEndOfDocument:"]
        for (input, name) in moves {
            for (flags, base) in [([] as UIKeyModifierFlags, name), (.alternate, wordMoves[name] ?? name), (.command, lineMoves[name] ?? name)] {
                for shift in [false, true] {
                    let full = shift ? base.replacingOccurrences(of: ":", with: "AndModifySelection:") : base
                    let c = UIKeyCommand(action: #selector(NoteView.textCommand(_:)), input: input, modifierFlags: shift ? flags.union(.shift) : flags, propertyList: full)
                    c.wantsPriorityOverSystemBehavior = true
                    commands.append(c)
                }
            }
        }
        commands.append(UIKeyCommand(action: #selector(NoteView.textCommand(_:)), input: "\u{8}", modifierFlags: .alternate, propertyList: "deleteWordBackward:"))
        commands.append(UIKeyCommand(action: #selector(NoteView.appCommand(_:)), input: UIKeyCommand.inputEscape, modifierFlags: [], propertyList: "escape"))
        commands.append(UIKeyCommand(action: #selector(NoteView.textCommand(_:)), input: UIKeyCommand.inputPageUp, modifierFlags: [], propertyList: "pageUp:"))
        commands.append(UIKeyCommand(action: #selector(NoteView.textCommand(_:)), input: UIKeyCommand.inputPageDown, modifierFlags: [], propertyList: "pageDown:"))
        return commands
    }()

    /// The model's commands, by menu, as key commands.
    static let menus: [(String, [UIKeyCommand])] = {
        guard let s = kolada_commands() else { return [] }
        defer { kolada_free_string(s) }
        var order: [String] = []
        var by: [String: [UIKeyCommand]] = [:]
        for line in String(cString: s).split(separator: "\n") {
            let f = line.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
            guard f.count == 5 else { continue }
            var flags: UIKeyModifierFlags = []
            for m in f[3].split(separator: " ") {
                switch m {
                case "cmd": flags.insert(.command)
                case "shift": flags.insert(.shift)
                case "alt": flags.insert(.alternate)
                case "ctrl": flags.insert(.control)
                default: break
                }
            }
            let c = UIKeyCommand(title: f[1], action: #selector(NoteView.appCommand(_:)), input: f[2], modifierFlags: flags, propertyList: f[4])
            if f[2].isEmpty {
                // No key of its own: a menu item only.
                by[f[0], default: []].append(UIKeyCommand(title: f[1], action: #selector(NoteView.appCommand(_:)), input: "", modifierFlags: [], propertyList: f[4]))
            } else {
                by[f[0], default: []].append(c)
            }
            if !order.contains(f[0]) { order.append(f[0]) }
        }
        return order.map { ($0, by[$0] ?? []) }
    }()

    /// The iPadOS menu bar: Kolada's File, Edit, Format, View and Go, in
    /// place of the system's items that take the same keys.
    static func build(_ builder: UIMenuBuilder) {
        guard builder.system == .main else { return }
        for id: UIMenu.Identifier in [.find, .newScene, .preferences, .textStyle, .undoRedo, .format, .font, .text, .spelling, .substitutions, .transformations, .speech] {
            builder.remove(menu: id)
        }
        for (name, commands) in menus {
            let menu = UIMenu(title: name, identifier: UIMenu.Identifier("app.kolada.\(name.lowercased())"), options: name == "Edit" || name == "File" ? .displayInline : [], children: commands)
            switch name {
            case "File": builder.insertChild(menu, atStartOfMenu: .file)
            case "Edit": builder.insertChild(menu, atStartOfMenu: .edit)
            case "View": builder.insertChild(UIMenu(title: "", options: .displayInline, children: commands), atStartOfMenu: .view)
            case "Format": builder.insertSibling(menu, afterMenu: .edit)
            default: builder.insertSibling(menu, afterMenu: .view)
            }
        }
    }
}
