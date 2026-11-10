package main

import (
    "context"
    "database/sql"
)

type Repository struct { db *sql.DB }

func (r *Repository) Save(ctx context.Context) error {
    _, err := r.db.Exec(`
        INSERT INTO audit_entries (id, value)
        VALUES (1, 'FROM not_a_table')
    `)
    return err
}

func load(db *sql.DB) (*sql.Rows, error) {
    return db.Query(`
        SELECT a.id FROM audit_entries a
        JOIN accounts b ON a.id = b.id
        -- JOIN comment_table ON true
        /* FROM another_comment */
    `)
}

func update(ctx context.Context, tx *sql.Tx) error {
    _, err := tx.ExecContext(ctx, "UPDATE accounts SET name = $1", "FROM argument_table")
    return err
}

func remove(ctx context.Context, conn *sql.Conn) error {
    _, err := conn.ExecContext(ctx, `DELETE FROM expired_entries WHERE id = 1`)
    return err
}

func count(ctx context.Context, db *sql.DB) *sql.Row {
    return db.QueryRowContext(ctx, `SELECT count(*) FROM accounts`)
}

func first(db *sql.DB) *sql.Row {
    return db.QueryRow(`SELECT id FROM audit_entries LIMIT 1`)
}

func contextual(ctx context.Context, db *sql.DB) (*sql.Rows, error) {
    return db.QueryContext(ctx, "SELECT id\nFROM accounts")
}

func ignored(db *sql.DB, query string) {
    db.Query(query)
    println("SELECT * FROM log_only")
}
