-- Add up migration script here
CREATE TABLE IF NOT EXISTS plugins (
    id SERIAL PRIMARY KEY,
    description TEXT NOT NULL,
    wasm_url TEXT NOT NULL
);
