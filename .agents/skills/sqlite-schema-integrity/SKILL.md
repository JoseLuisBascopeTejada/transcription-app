---
name: sqlite-schema-integrity
description: Use this skill whenever writing or editing code in src-tauri that touches the SQLite database — schema creation, migrations, insert/query functions for meetings/speakers/segments, or any rusqlite call. Trigger this even for small changes — this skill exists to prevent silent data-loss and query bugs from schema drift, since SQLite will not stop you from writing a query against a column that no longer exists in the way a compiled ORM would.
---

# SQLite Schema Integrity — One Source of Truth, Foreign Keys On

## The critical rule

**The schema in PLAN.md §3.3 is the single source of truth. Never modify a
table's columns inline in a query-writing task — schema changes are a
separate, explicit task with a migration, and every query-writing task must
paste the current authoritative schema into context rather than relying on
memory of it.**

SQLite does not enforce foreign keys by default — `PRAGMA foreign_keys = ON`
must be set on every connection, or `ON DELETE CASCADE` in the schema
silently does nothing and deleting a meeting leaves orphaned segments and
speakers behind.

### Wrong (do not do this)

```rust
fn get_connection() -> Connection {
    Connection::open("meetings.db").unwrap()   // ❌ foreign keys OFF by default —
                                                 //    CASCADE deletes silently no-op
}
```

### Right

```rust
fn get_connection(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open(db_path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS meetings (
            id INTEGER PRIMARY KEY,
            title TEXT NOT NULL,
            created_at TEXT NOT NULL,
            duration_seconds INTEGER,
            source_type TEXT CHECK(source_type IN ('live','file')),
            audio_path TEXT
        );

        CREATE TABLE IF NOT EXISTS speakers (
            id INTEGER PRIMARY KEY,
            meeting_id INTEGER REFERENCES meetings(id) ON DELETE CASCADE,
            label TEXT NOT NULL,
            embedding BLOB
        );

        CREATE TABLE IF NOT EXISTS segments (
            id INTEGER PRIMARY KEY,
            meeting_id INTEGER REFERENCES meetings(id) ON DELETE CASCADE,
            speaker_id INTEGER REFERENCES speakers(id),
            start_ms INTEGER NOT NULL,
            end_ms INTEGER NOT NULL,
            text TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_segments_meeting ON segments(meeting_id);
        "
    )?;
    Ok(())
}
```

## Checklist before finishing any task touching the database layer

- [ ] Does every new connection set `PRAGMA foreign_keys = ON`?
- [ ] Does the query match the exact column names/types in the schema above — not a remembered or assumed version from an earlier task?
- [ ] Are inserts wrapped in a transaction when writing a full meeting (meeting row + speaker rows + segment rows together), so a crash mid-write can't leave a meeting with no segments?
- [ ] Does any schema-altering change come with an explicit migration step (`ALTER TABLE` or a versioned migration file), never a silent `DROP TABLE` / recreate that would destroy existing user data?

## Migrations — never destructive by default

Since this is a local-first app storing irreplaceable user transcripts,
**no migration may drop or truncate an existing table** without an explicit,
separately-flagged task authorizing a breaking schema change with a
documented upgrade path for existing `.db` files. Additive changes
(`ALTER TABLE ... ADD COLUMN`) are safe to do routinely; anything else is
not routine.

## Failure signs to watch for

- Deleting a meeting from the UI but its segments still appearing in a
  search/history query — foreign keys were off.
- A query referencing a column name that doesn't match PLAN.md §3.3 exactly
  (e.g. `speaker_label` vs. the schema's `label` on the `speakers` table) —
  always cross-check column names literally, don't infer them.
- Partial meeting data after an app crash mid-save — writes not wrapped in
  a transaction.