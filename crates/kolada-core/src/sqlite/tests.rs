use super::*;

/// See testdata/make.py.
const FIXTURE: &[u8] = include_bytes!("../../testdata/sqlite.db");
const INTS: [i64; 10] = [0, 1, -1, 127, -129, 40000, -40000, 1 << 31, -(1 << 47), 1 << 62];

/// Row `i` of the plain table, as make.py wrote it.
fn plain_row(i: i64) -> Vec<Value> {
    let name = if i % 50 == 0 {
        format!("long {i} {}{}", "x".repeat(3000), "\u{0639}".repeat(500))
    } else {
        format!("row {i} {}", "\u{00E9}".repeat((i % 7) as usize))
    };
    let r = if i % 4 == 0 { Value::Null } else { Value::Real(i as f64 / 8.0) };
    let b = if i % 5 == 0 { Value::Blob(vec![(i % 256) as u8, 0, 255]) } else { Value::Null };
    let late = if i >= 290 { Value::Text("added".into()) } else { Value::Null };
    vec![Value::Int(i), Value::Text(name), Value::Int(INTS[i as usize % INTS.len()]), r, b, late]
}

#[test]
fn reads_every_row_through_a_tree_of_pages() {
    let db = Database::open(FIXTURE).unwrap();
    let t = db.table("PLAIN").unwrap();
    assert_eq!(t.columns, ["id", "name", "n", "r", "b", "late"]);
    assert_eq!(t.rows.len(), 300);
    for (i, row) in t.rows.iter().enumerate() {
        assert_eq!(row, &plain_row(i as i64), "row {i}");
    }
    assert_eq!(t.column("Name"), Some(1));
}

#[test]
fn a_column_that_is_the_rows_id_reads_as_it() {
    let db = Database::open(FIXTURE).unwrap();
    let t = db.table("keyed").unwrap();
    assert_eq!(t.columns, ["k", "v"]);
    let want: Vec<Vec<Value>> = (1..40).map(|k| vec![Value::Int(k * 3), Value::Text(format!("v{k}"))]).collect();
    assert_eq!(t.rows, want);
}

#[test]
fn text_cut_off_mid_character_loses_only_the_cut() {
    let db = Database::open(FIXTURE).unwrap();
    assert_eq!(db.table("cut").unwrap().rows, [[Value::Text("\u{0642}\u{0635}".into())]]);
    assert_eq!(utf8(b"ok \xFF end"), "ok \u{FFFD} end");
    assert_eq!(utf8(b"\xE2\x82"), "");
    assert_eq!(utf8("\u{20AC}".as_bytes()), "\u{20AC}");
    // A flag written as surrogates, then one cut between its surrogates.
    assert_eq!(utf8(b"\xED\xA0\xBC\xED\xB7\xB8 ok"), "\u{1F1F8} ok");
    assert_eq!(utf8(b"flag \xF0\x9F\x87\xB8\xED\xA0\xBC"), "flag \u{1F1F8}");
    assert_eq!(utf8(b"a \xED\xA0\xBC b"), "a \u{FFFD} b");
}

#[test]
fn refuses_what_it_cannot_read() {
    assert!(Database::open(b"not a database").is_err());
    let db = Database::open(FIXTURE).unwrap();
    assert!(db.table("missing").is_err());
    // Cut short: pages past the end are refused, not read.
    let short = &FIXTURE[..FIXTURE.len() / 3];
    assert!(Database::open(short).unwrap().table("plain").is_err());
    // Scrambled: an error or rows, never a panic.
    let mut scrambled = FIXTURE.to_vec();
    for (i, b) in scrambled.iter_mut().enumerate().skip(600) {
        if i % 97 == 0 {
            *b ^= 0x5A;
        }
    }
    let _ = Database::open(&scrambled).unwrap().table("plain");
}

#[test]
fn columns_come_from_the_statement() {
    assert_eq!(columns(r#"CREATE TABLE t ("a b" TEXT, [c] INT, `d`, e TEXT, UNIQUE(a, c))"#), (vec!["a b".into(), "c".into(), "d".into(), "e".into()], None));
    assert_eq!(columns("create table t (x text, id integer primary key)").1, Some(1));
    assert_eq!(columns("create table t (id integer, x text, primary key (id))").1, Some(0));
    assert_eq!(columns("create table t (id text primary key, x)").1, None);
    assert_eq!(columns("create table t (a numeric(10, 2), b check (b > 0))").0, ["a", "b"]);
}

/// Reads a real file and prints a digest of one table to compare with
/// sqlite3's (KOLADA_SQLITE=file, KOLADA_SQLITE_TABLE=table).
#[test]
#[ignore = "reads a file named by KOLADA_SQLITE"]
fn digest_of_a_real_file() {
    let path = std::env::var("KOLADA_SQLITE").unwrap();
    let table = std::env::var("KOLADA_SQLITE_TABLE").unwrap();
    let bytes = std::fs::read(path).unwrap();
    let t = Database::open(&bytes).unwrap().table(&table).unwrap();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for row in &t.rows {
        for v in row {
            let s = match v {
                Value::Null => "\u{1}".to_string(),
                Value::Int(i) => i.to_string(),
                Value::Real(r) => format!("{r:?}"),
                Value::Text(s) => s.clone(),
                Value::Blob(b) => b.iter().map(|x| format!("{x:02x}")).collect(),
            };
            for &b in s.as_bytes().iter().chain(b"\x1f") {
                h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
        h = (h ^ 0x1e).wrapping_mul(0x100_0000_01b3);
        if std::env::var_os("KOLADA_SQLITE_ROWS").is_some() {
            println!("ROW {h:016x}");
        }
    }
    println!("rows {} digest {h:016x}", t.rows.len());
}
