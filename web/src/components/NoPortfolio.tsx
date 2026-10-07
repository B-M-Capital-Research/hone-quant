/**
 * Shown instead of a page while the signed-in user cannot see any portfolio (typically a new
 * member): what a portfolio is, and a way to create the first one when allowed.
 */
import { For, Show } from "solid-js";
import { portfoliosText } from "@/i18n/portfolios";
import { canCreate } from "@/lib/portfolio";
import { Icon } from "./Icon";

export function NoPortfolio(props: { onCreate: () => void }) {
  const t = portfoliosText;
  return (
    <section class="card pf-empty" aria-labelledby="pf-empty-title">
      <span class="pf-empty-icon" aria-hidden="true">
        <Icon name="briefcase" size={24} />
      </span>
      <h1 id="pf-empty-title">{t().empty.title}</h1>
      <p class="pf-empty-body">{t().empty.body}</p>
      <div class="pf-empty-cols">
        <div>
          <div class="kicker">{t().empty.per_title}</div>
          <ul>
            <For each={t().empty.per_items}>{(item) => <li>{item}</li>}</For>
          </ul>
        </div>
        <div>
          <div class="kicker">{t().empty.shared_title}</div>
          <ul>
            <For each={t().empty.shared_items}>{(item) => <li>{item}</li>}</For>
          </ul>
        </div>
      </div>
      <Show when={canCreate()} fallback={<p class="muted small">{t().empty.ask_admin}</p>}>
        <button type="button" class="btn primary" onClick={() => props.onCreate()}>
          <Icon name="plus" size={16} /> {t().empty.create_first}
        </button>
      </Show>
    </section>
  );
}
