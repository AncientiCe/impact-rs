package main

import "database/sql"

func quoted(db *sql.DB) (*sql.Rows, error) {
    return db.Query("SELECT \"FROM\" AS label FROM \"public\".\"quoted_entries\"")
}
