// Apple Notes' notes, asked of Notes itself through its scripting
// dictionary (JavaScript for Automation): `osascript -l JavaScript
// applenotes.js <folder>`. First `total.txt`: how many notes can be read,
// how many are locked, and how many accounts there are. Then each note that
// is not locked, written to the folder as `note-<n>.txt` as it is read (n
// counts the notes saved, so a note that could not be saved leaves no gap):
// its fields apart by U+001F, its folders' names apart by U+001D. `done.txt`
// comes last, with how many notes were saved, locked and unreadable, so a
// reader knows the export ran to its end. Which folder is Notes' bin is told
// by its name in `applenotes.rs`, in whatever language Notes has. Only what
// Notes says is read; nothing in Notes is changed.
function run(argv) {
    ObjC.import('Foundation');
    var out = argv[0];
    var Notes = Application('Notes');
    var US = '\u001f', GS = '\u001d';
    var written = 0, locked = 0, failed = 0;
    var clean = function (s) { return String(s === null || s === undefined ? '' : s).replace(/[\u001c-\u001f]/g, ' '); };
    // Whether the file was saved.
    var save = function (name, text) {
        return !!$(text).writeToFileAtomicallyEncodingError(out + '/' + name, true, $.NSUTF8StringEncoding, null);
    };
    var millis = function (d) { try { return d ? String(d.getTime()) : '0'; } catch (e) { return '0'; } };
    var accounts = Notes.accounts();
    var seen = {};
    // A folder's path: its own name and its parents' up to the account.
    var path = function (f) {
        var p = [];
        var c = f;
        for (var k = 0; k < 32 && c; k++) {
            p.unshift(clean(c.name()));
            var up = null;
            try { up = c.container(); if (up.class() !== 'folder') { up = null; } } catch (e) { up = null; }
            c = up;
        }
        return p;
    };
    // Every folder, each once, with its account and path.
    var folders = [];
    var gather = function (f, account) {
        var id;
        try { id = f.id(); } catch (e) { return; }
        if (seen[id]) { return; }
        seen[id] = true;
        folders.push({ folder: f, account: clean(account), where: path(f).join(GS) });
        var inner = [];
        try { inner = f.folders(); } catch (e) {}
        for (var j = 0; j < inner.length; j++) { gather(inner[j], account); }
    };
    for (var a = 0; a < accounts.length; a++) {
        var account = '';
        try { account = accounts[a].name(); } catch (e) {}
        var top = [];
        try { top = accounts[a].folders(); } catch (e) { failed += 1; }
        for (var t = 0; t < top.length; t++) { gather(top[t], account); }
    }
    var open = 0, shut = 0;
    for (var c = 0; c < folders.length; c++) {
        try {
            var p = folders[c].folder.notes.passwordProtected();
            for (var q = 0; q < p.length; q++) { if (p[q]) { shut += 1; } else { open += 1; } }
        } catch (e) {}
    }
    save('total.txt', ['2', String(open), String(shut), String(accounts.length)].join(US));
    for (var k = 0; k < folders.length; k++) {
        var notes = folders[k].folder.notes;
        var ids = [], names = [], made = [], changed = [], prot = [];
        try {
            ids = notes.id(); names = notes.name(); made = notes.creationDate(); changed = notes.modificationDate(); prot = notes.passwordProtected();
        } catch (e) {
            failed += 1;
        }
        for (var i = 0; i < ids.length; i++) {
            if (prot[i]) { locked += 1; continue; }
            var note = notes.byId(ids[i]);
            var body;
            try { body = note.body(); } catch (e) { failed += 1; continue; }
            var attachments = 0;
            try { attachments = note.attachments.length; } catch (e) {}
            var record = ['2', folders[k].account, folders[k].where, clean(names[i]), millis(made[i]), millis(changed[i]), String(attachments), clean(body)].join(US);
            if (save('note-' + (written + 1) + '.txt', record)) { written += 1; } else { failed += 1; }
        }
    }
    save('done.txt', ['2', String(written), String(locked), String(failed)].join(US));
    return String(written);
}
