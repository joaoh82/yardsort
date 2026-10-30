#!/bin/sh
# Run once, when the PostgreSQL volume is first created: ZITADEL's database, and the server's own
# database and role. The password goes through a psql variable, so psql quotes it.
set -eu
psql -v ON_ERROR_STOP=1 -v password="$YARDSORT_DB_PASSWORD" --username "$POSTGRES_USER" <<'SQL'
CREATE DATABASE zitadel;
CREATE ROLE yardsort LOGIN PASSWORD :'password';
CREATE DATABASE yardsort OWNER yardsort;
SQL
