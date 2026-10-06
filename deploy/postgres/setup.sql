-- hone-quant on a PostgreSQL instance shared with honeclaw.
--
-- Run once as a superuser, e.g.  sudo -u postgres psql -v pw="'<strong password>'" < setup.sql
-- (feed the file on stdin: the postgres user often cannot read files in your home directory).
-- hone-quant creates its own schema objects on start-up (checksummed migrations); it never
-- touches honeclaw's tables. Pick ONE layout and keep the other commented out.

-- Layout A (recommended): a dedicated database on the shared instance -------------------------
CREATE ROLE hone_quant LOGIN PASSWORD :pw;
CREATE DATABASE hone_quant OWNER hone_quant;
REVOKE ALL ON DATABASE hone_quant FROM PUBLIC;
-- Connection: HONE_QUANT_DATABASE_URL=postgres://hone_quant:<password>@127.0.0.1:5432/hone_quant

-- Layout B: inside honeclaw's database, isolated in the hone_quant schema ----------------------
-- \connect honeclaw
-- CREATE ROLE hone_quant LOGIN PASSWORD :pw;
-- GRANT CONNECT ON DATABASE honeclaw TO hone_quant;
-- CREATE SCHEMA hone_quant AUTHORIZATION hone_quant;
-- -- No privileges on honeclaw's own schemas are needed (or wanted).
-- Connection: HONE_QUANT_DATABASE_URL=postgres://hone_quant:<password>@127.0.0.1:5432/honeclaw
