//! Just enough of SQLite's file format to read a table's rows: the recent
//! workspaces Zed keeps in its database. Read-only, without locking, with the
//! newer pages from the write-ahead log (`-wal`) over the file's, so it sees
//! what the editor wrote lately. Anything it can't make sense of (a page
//! being written as it's read, a format it doesn't know) is an error, not a
//! panic.
//!
//! The format: <https://www.sqlite.org/fileformat2.html>.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            _ => None,
        }
    }
}

/// A table's rows, each with a value per column (`NULL` for columns added
/// after it was written).
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl Table {
    /// The value of `column` in `row`.
    pub fn get<'a>(&self, row: &'a [Value], column: &str) -> Option<&'a Value> {
        let at = self
            .columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(column))?;
        row.get(at)
    }
}

pub struct Database {
    file: File,
    /// Page number -> where the latest committed copy of it starts in the WAL.
    wal: HashMap<u32, u64>,
    wal_file: Option<File>,
    page_size: usize,
    /// The page size less what's reserved at the end of each page.
    usable: usize,
    utf16: Option<Utf16>,
}

#[derive(Clone, Copy)]
enum Utf16 {
    Le,
    Be,
}

/// Most pages a table is read from: a guard against a damaged file whose
/// pages point round in a circle.
const MAX_PAGES: usize = 1_000_000;

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("sqlite: {what}"))
}

impl Database {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let mut header = [0u8; 100];
        file.read_exact(&mut header)?;
        if &header[..16] != b"SQLite format 3\0" {
            return Err(bad("not a database"));
        }
        let page_size = match u16::from_be_bytes([header[16], header[17]]) {
            1 => 65_536,
            n => usize::from(n),
        };
        if page_size < 512 || !page_size.is_power_of_two() {
            return Err(bad("odd page size"));
        }
        let usable = page_size - usize::from(header[20]);
        let utf16 = match u32::from_be_bytes([header[56], header[57], header[58], header[59]]) {
            2 => Some(Utf16::Le),
            3 => Some(Utf16::Be),
            _ => None,
        };
        let mut wal_path = path.as_os_str().to_owned();
        wal_path.push("-wal");
        let (wal, wal_file) = match File::open(&wal_path) {
            Ok(mut wal_file) => {
                let frames = read_wal(&mut wal_file, page_size).unwrap_or_default();
                (frames, Some(wal_file))
            }
            Err(_) => (HashMap::new(), None),
        };
        Ok(Self {
            file,
            wal,
            wal_file,
            page_size,
            usable,
            utf16,
        })
    }

    fn page(&mut self, number: u32) -> io::Result<Vec<u8>> {
        if number == 0 {
            return Err(bad("page 0"));
        }
        let mut page = vec![0; self.page_size];
        match (self.wal.get(&number), self.wal_file.as_mut()) {
            (Some(&at), Some(wal)) => {
                wal.seek(SeekFrom::Start(at))?;
                wal.read_exact(&mut page)?;
            }
            _ => {
                let at = u64::from(number - 1) * self.page_size as u64;
                self.file.seek(SeekFrom::Start(at))?;
                self.file.read_exact(&mut page)?;
            }
        }
        Ok(page)
    }

    /// Every row of the table `name`, or `None` if there's no such table.
    pub fn table(&mut self, name: &str) -> io::Result<Option<Table>> {
        let mut schema = Vec::new();
        self.walk(1, &mut schema)?;
        // sqlite_master: type, name, tbl_name, rootpage, sql.
        let found = schema.into_iter().find(|(_, row)| {
            row.first().and_then(Value::as_text) == Some("table")
                && row
                    .get(1)
                    .and_then(Value::as_text)
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
        });
        let Some((_, row)) = found else {
            return Ok(None);
        };
        let root = row
            .get(3)
            .and_then(Value::as_int)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| bad("no root page"))?;
        let sql = row.get(4).and_then(Value::as_text).unwrap_or_default();
        let (columns, rowid_column) = columns(sql);
        let mut rows = Vec::new();
        self.walk(root, &mut rows)?;
        let rows = rows
            .into_iter()
            .map(|(rowid, mut values)| {
                values.resize(columns.len().max(values.len()), Value::Null);
                // An INTEGER PRIMARY KEY is the rowid, stored as NULL.
                if let Some(at) = rowid_column
                    && let Some(value @ Value::Null) = values.get_mut(at)
                {
                    *value = Value::Int(rowid);
                }
                values
            })
            .collect();
        Ok(Some(Table { columns, rows }))
    }

    /// The rows of the table b-tree at `root`, with their rowids.
    fn walk(&mut self, root: u32, out: &mut Vec<(i64, Vec<Value>)>) -> io::Result<()> {
        let mut pending = vec![root];
        let mut seen = HashSet::new();
        while let Some(number) = pending.pop() {
            if !seen.insert(number) || seen.len() > MAX_PAGES {
                return Err(bad("pages go round in a circle"));
            }
            let page = self.page(number)?;
            // The first page starts with the file's header.
            let start = if number == 1 { 100 } else { 0 };
            let kind = *page.get(start).ok_or_else(|| bad("short page"))?;
            let cells = usize::from(be16(&page, start + 3)?);
            match kind {
                // Interior: each cell points at a child, and so does the header.
                5 => {
                    let header = start + 12;
                    for i in 0..cells {
                        let at = usize::from(be16(&page, header + 2 * i)?);
                        pending.push(be32(&page, at)?);
                    }
                    pending.push(be32(&page, start + 8)?);
                }
                // Leaf: the rows.
                13 => {
                    let header = start + 8;
                    for i in 0..cells {
                        let at = usize::from(be16(&page, header + 2 * i)?);
                        let (rowid, payload) = self.leaf_cell(&page, at)?;
                        out.push((rowid, self.record(&payload)?));
                    }
                }
                _ => return Err(bad("not a table page")),
            }
        }
        Ok(())
    }

    /// A table leaf cell's rowid and its whole payload, from overflow pages too.
    fn leaf_cell(&mut self, page: &[u8], at: usize) -> io::Result<(i64, Vec<u8>)> {
        let (size, n) = varint(page, at)?;
        let (rowid, m) = varint(page, at + n)?;
        let size = usize::try_from(size).map_err(|_| bad("huge cell"))?;
        let body = at + n + m;
        let u = self.usable;
        let local = local_size(size, u);
        let mut payload = page
            .get(body..body + local)
            .ok_or_else(|| bad("short cell"))?
            .to_vec();
        if local < size {
            let mut next = be32(page, body + local)?;
            let mut pages = 0;
            while payload.len() < size {
                pages += 1;
                if next == 0 || pages > MAX_PAGES {
                    return Err(bad("overflow ends early"));
                }
                let overflow = self.page(next)?;
                next = be32(&overflow, 0)?;
                let take = (size - payload.len()).min(u - 4);
                payload.extend_from_slice(overflow.get(4..4 + take).ok_or_else(|| bad("short"))?);
            }
        }
        Ok((rowid as i64, payload))
    }

    /// A record's values: a header of serial types, then the values.
    fn record(&self, payload: &[u8]) -> io::Result<Vec<Value>> {
        let (header_size, n) = varint(payload, 0)?;
        let header_size = usize::try_from(header_size).map_err(|_| bad("huge header"))?;
        let mut at = n;
        let mut body = header_size;
        let mut values = Vec::new();
        while at < header_size {
            let (serial, n) = varint(payload, at)?;
            at += n;
            let (value, size) = self.value(serial, payload, body)?;
            body += size;
            values.push(value);
        }
        Ok(values)
    }

    fn value(&self, serial: u64, payload: &[u8], at: usize) -> io::Result<(Value, usize)> {
        let int = |size: usize| -> io::Result<(Value, usize)> {
            let bytes = payload
                .get(at..at + size)
                .ok_or_else(|| bad("short value"))?;
            // Big-endian, two's complement.
            let mut n: i64 = if bytes.first().is_some_and(|b| b & 0x80 != 0) {
                -1
            } else {
                0
            };
            for &b in bytes {
                n = (n << 8) | i64::from(b);
            }
            Ok((Value::Int(n), size))
        };
        Ok(match serial {
            0 => (Value::Null, 0),
            1..=4 => int(serial as usize)?,
            5 => int(6)?,
            6 => int(8)?,
            7 => {
                let bytes = payload.get(at..at + 8).ok_or_else(|| bad("short value"))?;
                let bits = u64::from_be_bytes(bytes.try_into().map_err(|_| bad("short"))?);
                (Value::Real(f64::from_bits(bits)), 8)
            }
            8 => (Value::Int(0), 0),
            9 => (Value::Int(1), 0),
            n if n >= 12 => {
                let size = usize::try_from((n - 12) / 2).map_err(|_| bad("huge value"))?;
                let bytes = payload
                    .get(at..at + size)
                    .ok_or_else(|| bad("short value"))?;
                let value = if n % 2 == 0 {
                    Value::Blob(bytes.to_vec())
                } else {
                    Value::Text(self.text(bytes))
                };
                (value, size)
            }
            _ => return Err(bad("unknown serial type")),
        })
    }

    fn text(&self, bytes: &[u8]) -> String {
        let Some(order) = self.utf16 else {
            return String::from_utf8_lossy(bytes).into_owned();
        };
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| match order {
                Utf16::Le => u16::from_le_bytes(pair),
                Utf16::Be => u16::from_be_bytes(pair),
            })
            .collect();
        String::from_utf16_lossy(&units)
    }
}

/// How much of a table leaf cell's payload of `size` is on the page itself,
/// the rest going to overflow pages, for pages with `u` usable bytes.
fn local_size(size: usize, u: usize) -> usize {
    let max = u - 35;
    if size <= max {
        return size;
    }
    let min = (u - 12) * 32 / 255 - 23;
    let k = min + (size - min) % (u - 4);
    if k <= max { k } else { min }
}

/// The pages in the WAL that belong to committed transactions, latest copy
/// of each, as where they start in the file.
fn read_wal(wal: &mut File, page_size: usize) -> io::Result<HashMap<u32, u64>> {
    let mut header = [0u8; 32];
    wal.read_exact(&mut header)?;
    let magic = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    if magic & !1 != 0x377f_0682 {
        return Err(bad("not a WAL file"));
    }
    if u32::from_be_bytes([header[8], header[9], header[10], header[11]]) as usize != page_size {
        return Err(bad("WAL page size"));
    }
    let salts = &header[16..24];
    let mut committed = HashMap::new();
    let mut pending: HashMap<u32, u64> = HashMap::new();
    let mut at = 32u64;
    let mut frame = [0u8; 24];
    loop {
        wal.seek(SeekFrom::Start(at))?;
        if wal.read_exact(&mut frame).is_err() {
            break;
        }
        // A frame from before the log was last restarted.
        if &frame[8..16] != salts {
            break;
        }
        let page = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]);
        pending.insert(page, at + 24);
        // The last frame of a transaction says how big the file is now.
        if frame[4..8] != [0, 0, 0, 0] {
            committed.extend(pending.drain());
        }
        at += 24 + page_size as u64;
    }
    Ok(committed)
}

/// The column names in a `CREATE TABLE` statement, and which one is an
/// INTEGER PRIMARY KEY (the rowid).
fn columns(sql: &str) -> (Vec<String>, Option<usize>) {
    let (Some(open), Some(close)) = (sql.find('('), sql.rfind(')')) else {
        return (Vec::new(), None);
    };
    let Some(body) = sql.get(open + 1..close) else {
        return (Vec::new(), None);
    };
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in body.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&body[start..]);
    let mut names = Vec::new();
    let mut rowid = None;
    for part in parts {
        let part = part.trim();
        let first = part.split_whitespace().next().unwrap_or_default();
        let constraint = ["primary", "foreign", "unique", "check", "constraint"]
            .iter()
            .any(|k| first.eq_ignore_ascii_case(k));
        if constraint || first.is_empty() {
            continue;
        }
        let words: Vec<String> = part
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect();
        if words.get(1).map(String::as_str) == Some("integer")
            && words.windows(2).any(|w| w == ["primary", "key"])
        {
            rowid = Some(names.len());
        }
        names.push(first.trim_matches(['"', '`', '[', ']']).to_string());
    }
    (names, rowid)
}

fn be16(bytes: &[u8], at: usize) -> io::Result<u16> {
    let b = bytes.get(at..at + 2).ok_or_else(|| bad("short page"))?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn be32(bytes: &[u8], at: usize) -> io::Result<u32> {
    let b = bytes.get(at..at + 4).ok_or_else(|| bad("short page"))?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// A varint (1 to 9 bytes, 7 bits each but the ninth) and how many bytes it took.
fn varint(bytes: &[u8], at: usize) -> io::Result<(u64, usize)> {
    let mut n = 0u64;
    for i in 0..9 {
        let b = *bytes.get(at + i).ok_or_else(|| bad("short varint"))?;
        if i == 8 {
            return Ok(((n << 8) | u64::from(b), 9));
        }
        n = (n << 7) | u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            return Ok((n, i + 1));
        }
    }
    unreachable!("returns by the ninth byte")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints() {
        assert_eq!(varint(&[0x05], 0).unwrap(), (5, 1));
        assert_eq!(varint(&[0x81, 0x00], 0).unwrap(), (128, 2));
        assert_eq!(varint(&[0xff; 9], 0).unwrap(), (u64::MAX, 9));
        assert!(varint(&[0x81], 0).is_err());
    }

    #[test]
    fn column_names() {
        let (names, rowid) = columns(
            "CREATE TABLE workspaces (\n  workspace_id INTEGER PRIMARY KEY,\n  paths TEXT,\n  \
             remote_connection_id INTEGER REFERENCES remote_connections (id),\n  \
             timestamp TEXT DEFAULT CURRENT_TIMESTAMP NOT NULL,\n  FOREIGN KEY(x) REFERENCES y(id)\n) STRICT",
        );
        assert_eq!(
            names,
            ["workspace_id", "paths", "remote_connection_id", "timestamp"]
        );
        assert_eq!(rowid, Some(0));
        let (names, rowid) = columns("CREATE TABLE \"t\" (\"a b\" TEXT, c, PRIMARY KEY (c))");
        assert_eq!(names, ["a", "c"]);
        assert_eq!(rowid, None);
    }

    #[test]
    fn payloads_spill_over_as_sqlite_says() {
        // 4096-byte pages: up to 4061 bytes stay on the page.
        assert_eq!(local_size(100, 4096), 100);
        assert_eq!(local_size(4061, 4096), 4061);
        assert_eq!(local_size(5000, 4096), 908);
        assert!(local_size(100_000, 4096) <= 4061);
    }

    /// With the sqlite3 command-line tool installed: a database it makes,
    /// rows on several pages and in the log, a long value on overflow pages.
    #[test]
    fn reads_what_sqlite_wrote() {
        let dir = std::env::temp_dir().join(format!("proj-sqlite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("db.sqlite");
        let long = "x".repeat(10_000);
        let script = format!(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;\n\
             CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, n INTEGER, r REAL, b BLOB);\n\
             WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM c WHERE i < 500)\n\
             INSERT INTO t (name, n, r) SELECT 'row ' || i, i * -1000, i / 4.0 FROM c;\n\
             INSERT INTO t (name, b) VALUES ('{long}', x'00ff');\n\
             ALTER TABLE t ADD COLUMN later TEXT;\n"
        );
        let made = std::process::Command::new("sqlite3")
            .arg(&db)
            .arg(script)
            .output()
            .is_ok_and(|o| o.status.success());
        if !made {
            return; // No sqlite3 here.
        }
        let mut database = Database::open(&db).unwrap();
        let table = database.table("t").unwrap().unwrap();
        assert_eq!(table.columns, ["id", "name", "n", "r", "b", "later"]);
        assert_eq!(table.rows.len(), 501);
        let row = table.rows.iter().find(|r| r[0] == Value::Int(250)).unwrap();
        assert_eq!(row[1], Value::Text("row 250".into()));
        assert_eq!(row[2], Value::Int(-250_000));
        assert_eq!(row[3], Value::Real(62.5));
        assert_eq!(row[5], Value::Null);
        let last = table.rows.iter().find(|r| r[0] == Value::Int(501)).unwrap();
        assert_eq!(last[1], Value::Text(long));
        assert_eq!(last[4], Value::Blob(vec![0, 255]));
        assert!(database.table("missing").unwrap().is_none());

        // Changes still in the log, while sqlite3 has the database open: the
        // log's pages win over the file's.
        use std::io::{BufRead, Write};
        let mut open = std::process::Command::new("sqlite3")
            .arg(&db)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = open.stdin.take().unwrap();
        writeln!(
            stdin,
            "PRAGMA wal_autocheckpoint=0;\n\
             UPDATE t SET name = 'changed' WHERE id = 1;\n\
             INSERT INTO t (name, later) VALUES ('from the log', 'yes');\n\
             .print done"
        )
        .unwrap();
        // The pragma prints its value first.
        let printed = std::io::BufReader::new(open.stdout.take().unwrap()).lines();
        let done = printed
            .map_while(Result::ok)
            .any(|line| line.trim() == "done");
        assert!(done);
        assert!(dir.join("db.sqlite-wal").is_file());
        let table = Database::open(&db).unwrap().table("t").unwrap().unwrap();
        assert_eq!(table.rows.len(), 502);
        let name = |id: i64| {
            let row = table.rows.iter().find(|r| r[0] == Value::Int(id)).unwrap();
            row[1].clone()
        };
        assert_eq!(name(1), Value::Text("changed".into()));
        assert_eq!(name(502), Value::Text("from the log".into()));
        drop(stdin);
        open.wait().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
