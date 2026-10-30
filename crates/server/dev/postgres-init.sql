-- Run once, when the PostgreSQL volume is first created. ZITADEL keeps its data in `zitadel`, the
-- server in `yardsort`, and the server's tests make a schema each in `yardsort_test`.
CREATE DATABASE zitadel;
CREATE DATABASE yardsort;
CREATE DATABASE yardsort_test;
