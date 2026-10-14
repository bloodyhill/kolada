"""Makes the SQLite files the tests read: python make.py (from this folder).

sqlite.db: small pages, so a few hundred rows need a tree of pages and long
text needs overflow pages; every kind of value; a column that is the row's
id; a column added after rows were written; text cut off mid-character.

notes.nbk: an export of Notes (github.com/nuttyartist/notes), as its
schema has it: folders (the root, the trash, the default one, a folder and
a folder in it), notes in each, tags, a placeholder note, a note that is
only its title.
"""
import os
import sqlite3

here = os.path.dirname(os.path.abspath(__file__))


def fresh(name):
    path = os.path.join(here, name)
    if os.path.exists(path):
        os.remove(path)
    return sqlite3.connect(path)


INTS = [0, 1, -1, 127, -129, 40000, -40000, 2**31, -(2**47), 2**62]


def plain_row(i):
    """Row i of the plain table; the Rust test builds the same."""
    name = f"row {i} " + "é" * (i % 7)
    if i % 50 == 0:
        name = f"long {i} " + "x" * 3000 + "ع" * 500
    n = INTS[i % len(INTS)]
    r = None if i % 4 == 0 else i / 8
    b = bytes([i % 256, 0, 255]) if i % 5 == 0 else None
    return i, name, n, r, b


db = fresh("sqlite.db")
db.execute("pragma page_size = 512")
db.execute('create table "plain" ("id" INTEGER NOT NULL, "name" TEXT, n INTEGER, [r] REAL, `b` BLOB, unique(id, name))')
db.executemany("insert into plain values (?, ?, ?, ?, ?)", [plain_row(i) for i in range(300)])
db.execute("alter table plain add column late TEXT")
db.execute("update plain set late = 'added' where id >= 290")
db.execute("create table keyed (k integer primary key, v text)")
db.executemany("insert into keyed values (?, ?)", [(k * 3, f"v{k}") for k in range(1, 40)])
db.execute("create table cut (v text)")
# An Arabic word cut after the first byte of its last letter.
db.execute("insert into cut values (cast(x'd982d8b5d8' as text))")
db.commit()
db.execute("vacuum")
db.close()

db = fresh("notes.nbk")
db.executescript("""
CREATE TABLE "metadata" ("key" TEXT NOT NULL, "value" INTEGER NOT NULL);
CREATE TABLE "node_table" ("id" INTEGER NOT NULL, "title" TEXT, "creation_date" INTEGER NOT NULL DEFAULT 0,
  "modification_date" INTEGER NOT NULL DEFAULT 0, "deletion_date" INTEGER NOT NULL DEFAULT 0, "content" TEXT,
  "node_type" INTEGER NOT NULL, "parent_id" INTEGER NOT NULL, "relative_position" INTEGER NOT NULL,
  "scrollbar_position" INTEGER NOT NULL, "absolute_path" TEXT NOT NULL, "is_pinned_note" INTEGER NOT NULL DEFAULT 0,
  "relative_position_an" INTEGER NOT NULL, "child_notes_count" INTEGER NOT NULL);
CREATE TABLE "tag_relationship" ("node_id" INTEGER NOT NULL, "tag_id" INTEGER NOT NULL, UNIQUE(node_id, tag_id));
CREATE TABLE "tag_table" ("id" INTEGER NOT NULL, "name" TEXT NOT NULL, "color" TEXT NOT NULL,
  "child_notes_count" INTEGER NOT NULL, "relative_position" INTEGER NOT NULL);
""")
FOLDER, NOTE = 1, 0
DAY = 86_400_000
T0 = 1_665_419_890_626  # 2022-10-10


def node(i, title, kind, parent, path, content=None, created=0, modified=0, deleted=-1):
    db.execute(
        "insert into node_table values (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, 0, ?, 0)",
        (i, title, created, modified, deleted, content, kind, parent, i, path, i),
    )


node(0, "/", FOLDER, -1, "/0")
node(1, "Trash", FOLDER, 0, "/0/1")
node(2, "Notes", FOLDER, 0, "/0/2")
node(3, "Work", FOLDER, 0, "/0/3")
node(4, "Plans: 2026", FOLDER, 3, "/0/3/4")
node(10, "Groceries", NOTE, 2, "/0/2/10", "Groceries\n- milk\n- bread", T0, T0 + DAY)
node(11, "Groceries", NOTE, 2, "/0/2/11", "Groceries\nagain, the week after", T0 + 7 * DAY, T0 + 8 * DAY)
node(12, "What: now?", NOTE, 3, "/0/3/12", "# What: now?\n**Soon**", T0 + 2 * DAY, T0 + 30 * DAY)
node(13, "Launch", NOTE, 4, "/0/3/4/13", "Launch\nthe plan", T0 + 3 * DAY, T0 + 3 * DAY)
node(14, "Old idea", NOTE, 1, "/0/1/14", "Old idea\nthrown away", T0, T0, T0 + 9 * DAY)
node(15, "New Note", NOTE, 2, "/0/2/15", "", T0 + 4 * DAY, T0 + 4 * DAY)
node(16, "Only a title, nothing written under it", NOTE, 2, "/0/2/16", "", T0 + 5 * DAY, T0 + 6 * DAY)
node(17, "Tagged", NOTE, 2, "/0/2/17", "Tagged\nwith two", T0 + 6 * DAY, T0 + 6 * DAY)
db.executemany("insert into tag_table values (?, ?, '#000', 1, ?)", [(1, "reading list", 0), (2, "ideas", 1)])
db.executemany("insert into tag_relationship values (?, ?)", [(17, 1), (17, 2)])
db.executemany("insert into metadata values (?, ?)", [("next_node_id", 18), ("next_tag_id", 3)])
db.commit()
db.execute("vacuum")
db.close()
print("made sqlite.db and notes.nbk")
