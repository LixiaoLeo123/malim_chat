#!/usr/bin/env python3
"""Build the read-only local English-Chinese SQLite index from ECDICT CSV."""

import csv
import sqlite3
import sys
from pathlib import Path


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: build_ecdict_sqlite.py ECDICT.csv OUTPUT.sqlite")
    source, target = map(Path, sys.argv[1:])
    if target.exists():
        target.unlink()
    connection = sqlite3.connect(target)
    connection.executescript("""
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        CREATE TABLE entries (
            word TEXT PRIMARY KEY COLLATE NOCASE,
            phonetic TEXT NOT NULL, definition TEXT NOT NULL, translation TEXT NOT NULL,
            pos TEXT NOT NULL, collins INTEGER NOT NULL, oxford INTEGER NOT NULL,
            tags TEXT NOT NULL, bnc INTEGER NOT NULL, frequency INTEGER NOT NULL,
            exchange TEXT NOT NULL, detail TEXT NOT NULL
        );
        CREATE INDEX entries_word_lower ON entries(word COLLATE NOCASE);
    """)
    with source.open("r", encoding="utf-8", newline="") as stream:
        reader = csv.DictReader(stream)
        rows = []
        for record in reader:
            rows.append((
                record["word"], record["phonetic"] or "", record["definition"] or "",
                record["translation"] or "", record["pos"] or "", int(record["collins"] or 0),
                int(record["oxford"] or 0), record["tag"] or "", int(record["bnc"] or 0),
                int(record["frq"] or 0), record["exchange"] or "", record["detail"] or "",
            ))
            if len(rows) == 2000:
                connection.executemany("INSERT OR REPLACE INTO entries VALUES (?,?,?,?,?,?,?,?,?,?,?,?)", rows)
                rows.clear()
        if rows:
            connection.executemany("INSERT OR REPLACE INTO entries VALUES (?,?,?,?,?,?,?,?,?,?,?,?)", rows)
    connection.commit()
    connection.execute("VACUUM")
    connection.close()


if __name__ == "__main__":
    main()
