/**
 * Workbench state: the starting point, the edited draft, validation (client rules mirrored from
 * the core plus 422 field errors from the server) and preview runs. Created once per page so
 * edits survive switching tabs.
 */
import { createMemo, createSignal } from "solid-js";
import { ApiError, api } from "@/lib/api";
import type { FieldError, Preview, StrategyParams } from "@/lib/types";
import { type Issue, cleanParams, clone, diffPaths, fieldForIssue, sameParams, setPath, validateParams } from "./params";

export type SourceKind = "active" | "version" | "preset";

export interface Source {
  key: string;
  kind: SourceKind;
  versionId?: number;
  presetId: string;
  name: string;
  params: StrategyParams;
}

export interface PreviewRun {
  /** Exactly what was sent. */
  params: StrategyParams;
  result: Preview;
  /** The active version's targets at the same moment, when the parameters differ from it. */
  active: Preview | null;
  activeFailed: boolean;
  sameAsActive: boolean;
  /** Current portfolio weights by symbol (from the dashboard valuation). */
  current: Record<string, number> | null;
  ranAt: number;
}

export function createWorkbench() {
  const [source, setSource] = createSignal<Source | null>(null);
  const [draft, setDraft] = createSignal<StrategyParams | null>(null);
  const [serverIssues, setServerIssues] = createSignal<Issue[]>([]);
  const [general, setGeneral] = createSignal<string[]>([]);
  const [run, setRun] = createSignal<PreviewRun | null>(null);
  const [running, setRunning] = createSignal(false);
  const [runError, setRunError] = createSignal<unknown>(null);
  const [saved, setSaved] = createSignal<{ id: number; activated: boolean } | null>(null);

  const clientIssues = createMemo<Issue[]>(() => {
    const d = draft();
    return d ? validateParams(d) : [];
  });
  const issues = createMemo<Issue[]>(() => {
    const own = clientIssues();
    const fromServer = serverIssues().filter((s) => !own.some((c) => c.path === s.path && c.code === s.code));
    return [...own, ...fromServer];
  });
  const changed = createMemo<string[]>(() => {
    const d = draft();
    const s = source();
    return d && s ? diffPaths(d, s.params) : [];
  });
  const outdated = createMemo(() => {
    const r = run();
    const d = draft();
    return !!r && !!d && !sameParams(cleanParams(d), r.params);
  });

  const load = (src: Source) => {
    setSource(src);
    setDraft(clone(src.params));
    setServerIssues([]);
    setGeneral([]);
    setSaved(null);
    setRun(null);
    setRunError(null);
  };

  /** Re-points the starting point without touching the draft (after saving it as a version). */
  const rebase = (src: Source) => {
    setSource(src);
  };

  const update = (path: string, value: unknown) => {
    setDraft((d) => (d ? setPath(d, path, value) : d));
    const field = fieldForIssue(path)?.path;
    setServerIssues((list) => list.filter((i) => fieldForIssue(i.path)?.path !== field));
    setGeneral([]);
    setSaved(null);
  };

  const revert = () => {
    const s = source();
    if (!s) return;
    setDraft(clone(s.params));
    setServerIssues([]);
    setGeneral([]);
  };

  /** Applies a 422 response to the form; returns true when it was a validation error. */
  const absorbServerError = (error: unknown): boolean => {
    if (error instanceof ApiError && error.code === "validation" && error.fields?.length) {
      const mapped: Issue[] = [];
      const loose: string[] = [];
      for (const f of error.fields as FieldError[]) {
        if (f.path && fieldForIssue(f.path)) mapped.push({ path: f.path, code: f.code, min: f.min, max: f.max, message: f.message });
        else loose.push(f.message || f.path);
      }
      setServerIssues(mapped);
      setGeneral(loose);
      return true;
    }
    return false;
  };

  /**
   * Runs the preview for the draft, plus the active version for comparison when it differs.
   * Resolves to "invalid" when the server rejected the parameters (issues are then on the form).
   */
  const preview = async (active: StrategyParams | null): Promise<"ok" | "invalid" | "error" | "busy"> => {
    const d = draft();
    if (!d || running()) return "busy";
    const params = cleanParams(d);
    setRunning(true);
    setRunError(null);
    const sameAsActive = !!active && sameParams(params, active);
    try {
      const [main, comparison, dashboard] = await Promise.allSettled([
        api.previewStrategy(params),
        active && !sameAsActive ? api.previewStrategy(active) : Promise.resolve(null),
        api.dashboard(),
      ]);
      if (main.status === "rejected") {
        if (absorbServerError(main.reason)) return "invalid";
        setRunError(main.reason);
        return "error";
      }
      const current: Record<string, number> | null =
        dashboard.status === "fulfilled" ? Object.fromEntries(dashboard.value.valuation.positions.map((p) => [p.symbol, p.weight])) : null;
      setRun({
        params,
        result: main.value,
        active: comparison.status === "fulfilled" ? comparison.value : null,
        activeFailed: comparison.status === "rejected",
        sameAsActive,
        current,
        ranAt: Date.now(),
      });
      return "ok";
    } finally {
      setRunning(false);
    }
  };

  return {
    source,
    draft,
    issues,
    clientIssues,
    serverIssues,
    general,
    changed,
    run,
    running,
    runError,
    outdated,
    saved,
    setSaved,
    load,
    rebase,
    update,
    revert,
    preview,
    absorbServerError,
    setGeneral,
  };
}

export type Workbench = ReturnType<typeof createWorkbench>;
