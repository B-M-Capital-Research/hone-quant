/**
 * Pieces of the AI sector view shared by the AI page and the plan detail: the stance chip and the
 * per-plan record of what the view did.
 */
import { A } from "@solidjs/router";
import { For, Show } from "solid-js";
import { locale, tpl } from "@/i18n";
import { aiText } from "@/i18n/ai";
import { plansText } from "@/i18n/plans";
import { fmtNum, fmtPct } from "@/lib/format";
import type { AiEvidence, AiSectorOpinion, PlanAi, Sector } from "@/lib/types";
import { sectorLabel } from "../overview/util";

export function stanceText(stance: number): string {
  return aiText().stance[String(stance)] ?? String(stance);
}

/** −2 … +2 as a coloured chip: green leans in, orange/red leans away. */
export function StanceChip(props: { stance: number }) {
  const color = () => (props.stance >= 2 ? "green" : props.stance === 1 ? "green" : props.stance === -1 ? "orange" : props.stance <= -2 ? "red" : "outline");
  return (
    <span class={`chip ${color()} nowrap`}>
      {props.stance > 0 ? "+" : props.stance < 0 ? "−" : ""}
      {Math.abs(props.stance)} · {stanceText(props.stance)}
    </span>
  );
}

const zh = () => locale() === "zh";

export function opinionText(o: AiSectorOpinion, field: "rationale" | "risks"): string {
  const first = zh() ? o[`${field}_zh`] : o[`${field}_en`];
  return first || o[`${field}_zh`] || o[`${field}_en`] || "";
}

export function evidenceNote(e: AiEvidence): string {
  return (zh() ? e.note_zh || e.note_en : e.note_en || e.note_zh) || e.ref;
}

/** One sector's evidence, risks and validation flags. */
export function OpinionDetail(props: { opinion: AiSectorOpinion }) {
  const a = aiText;
  return (
    <div class="ai-detail">
      <Show when={props.opinion.proposed_stance != null}>
        <p class="xs ai-flag">{tpl(a().latest.proposed, { stance: stanceText(props.opinion.proposed_stance!) })}</p>
      </Show>
      <Show when={props.opinion.evidence.length}>
        <div class="xs muted">{a().latest.evidence}</div>
        <ul class="ai-evidence">
          <For each={props.opinion.evidence}>
            {(e) => (
              <li>
                <span class="chip outline">{a().source[e.source] ?? e.source}</span>
                <span class="num muted">{e.date ?? "—"}</span>
                <span>{evidenceNote(e)}</span>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <Show when={opinionText(props.opinion, "risks")}>
        <p class="xs">
          <span class="muted">{a().latest.risks}：</span>
          {opinionText(props.opinion, "risks")}
        </p>
      </Show>
      <Show when={props.opinion.flags.filter((f) => f !== "insufficient_evidence").length}>
        <div class="pill-list">
          <For each={props.opinion.flags.filter((f) => f !== "insufficient_evidence")}>{(f) => <span class="chip yellow">{a().flags[f] ?? f}</span>}</For>
        </div>
      </Show>
    </div>
  );
}

/** What the AI sector view did to one plan (`diagnostics.ai`). */
export function PlanAiPanel(props: { ai: PlanAi; sectors?: Sector[] }) {
  const a = aiText;
  const t = plansText;
  const statusChip = () => {
    switch (props.ai.status) {
      case "live":
        return <span class="chip green">{a().plan.status_live}</span>;
      case "shadow":
        return <span class="chip blue">{a().plan.status_shadow}</span>;
      default:
        return <span class="chip outline">{a().plan.status_unavailable}</span>;
    }
  };
  const note = () => (props.ai.status === "live" ? a().plan.live_note : props.ai.status === "shadow" ? a().plan.shadow_note : a().plan.unavailable_note);
  const rows = () => [...props.ai.sectors].sort((x, y) => Math.abs(y.weight_with - y.weight_without) - Math.abs(x.weight_with - x.weight_without));
  return (
    <div class="ai-plan">
      <div class="row wrap ai-plan-head">
        {statusChip()}
        <Show when={props.ai.view_id != null}>
          <A href={`/ai?view=${props.ai.view_id}`} class="xs">
            {tpl(a().plan.view, { id: props.ai.view_id! })}
          </A>
          <span class="xs muted">{tpl(a().plan.as_of, { date: props.ai.as_of ?? "—" })}</span>
          <span class="xs muted">{props.ai.model}</span>
          <span class="xs muted">{tpl(a().plan.active_share, { v: fmtPct(props.ai.active_share, { dp: 1 }) })}</span>
        </Show>
      </div>
      <p class="muted xs tab-note">{note()}</p>
      <Show when={props.ai.sectors.length}>
        <div class="table-wrap">
          <table class="table compact">
            <thead>
              <tr>
                <th>{t().detail.col_sector}</th>
                <th>{a().latest.col_stance}</th>
                <th class="r">{a().latest.col_confidence}</th>
                <th class="r">{a().plan.col_multiplier}</th>
                <th class="r">{a().plan.col_without}</th>
                <th class="r">{a().plan.col_with}</th>
                <th class="r">{a().plan.col_delta}</th>
              </tr>
            </thead>
            <tbody>
              <For each={rows()}>
                {(s) => {
                  const delta = () => s.weight_with - s.weight_without;
                  return (
                    <tr>
                      <td>{sectorLabel(props.sectors, s.sector)}</td>
                      <td>
                        <StanceChip stance={s.stance} />
                      </td>
                      <td class="r num">{s.stance === 0 && s.confidence === 0 ? "—" : fmtPct(s.confidence, { dp: 0 })}</td>
                      <td class="r num">×{fmtNum(s.multiplier, 3)}</td>
                      <td class="r num" classList={{ b: props.ai.status !== "live" }}>
                        {fmtPct(s.weight_without, { dp: 1 })}
                      </td>
                      <td class="r num" classList={{ b: props.ai.status === "live" }}>
                        {fmtPct(s.weight_with, { dp: 1 })}
                      </td>
                      <td class={`r num ${Math.abs(delta()) < 0.0005 ? "muted" : delta() > 0 ? "up" : "down"}`}>
                        {Math.abs(delta()) < 0.0005 ? "0.0%" : fmtPct(delta(), { dp: 1, sign: true })}
                      </td>
                    </tr>
                  );
                }}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
}
