-- hone-quant schema v3: AI sector views.
--
-- One row per attempt to form a view: the material the model read (ontology digest, market
-- statistics, earnings), its raw answer and the validated stances. Plans reference the view they
-- used (or recorded in shadow) by id in their diagnostics, so every AI influence on a plan can be
-- traced back to the exact input and answer.

CREATE TABLE ai_views (
    id              BIGSERIAL PRIMARY KEY,
    -- Last close the material includes.
    as_of           DATE NOT NULL,
    -- `running` is written before the model is called, so every call counts toward the cap.
    status          TEXT NOT NULL CHECK (status IN ('running', 'ok', 'failed')),
    prompt_version  TEXT NOT NULL,
    -- Model requested, and the one the endpoint reported.
    model           TEXT NOT NULL DEFAULT '',
    material        JSONB NOT NULL DEFAULT '{}'::jsonb,
    material_hash   TEXT NOT NULL DEFAULT '',
    -- Validated `ViewOutput` (status ok).
    output          JSONB,
    raw_response    TEXT,
    error           TEXT,
    usage           JSONB,
    latency_ms      INTEGER,
    created_by      TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT app_now()
);
CREATE INDEX ai_views_created_idx ON ai_views (created_at DESC);
CREATE INDEX ai_views_ok_idx ON ai_views (as_of DESC, id DESC) WHERE status = 'ok';
