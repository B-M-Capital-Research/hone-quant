/**
 * The portfolio switcher in the top bar: the current portfolio's name (with its owner when it is
 * someone else's) and a menu of the visible portfolios with NAV, total return and automation
 * mode, followed by "New portfolio…" and "Manage portfolios". Arrow keys move through the menu,
 * Escape closes it and returns focus to the button.
 */
import { A } from "@solidjs/router";
import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { portfoliosText } from "@/i18n/portfolios";
import { fmtMoney } from "@/lib/format";
import { canCreate, currentPortfolio, loadPortfolios, noPortfolio, portfolios, portfoliosError, selectPortfolio } from "@/lib/portfolio";
import { ownerKind } from "@/lib/portfolio-select";
import { me } from "@/lib/session";
import type { AutomationMode, Portfolio } from "@/lib/types";
import { Icon } from "./Icon";
import { Pct } from "./ui";

const MODE_TONE: Record<AutomationMode, string> = { auto: "green", approval: "blue", paused: "yellow" };

export function ModeChip(props: { mode: AutomationMode }) {
  return (
    <span class={`chip ${MODE_TONE[props.mode]}`} title={common().mode[`${props.mode}_hint` as const]}>
      <Icon name={props.mode === "paused" ? "pause" : "zap"} size={11} />
      {common().mode[props.mode]}
    </span>
  );
}

/** "Shared · administrators", "Yours" or the owner's name. */
export function ownerLabel(portfolio: Portfolio): string {
  const kind = ownerKind(portfolio, me()?.username);
  if (kind === "shared") return portfoliosText().owner.shared;
  if (kind === "own") return portfoliosText().owner.own;
  return portfolio.owner_name || portfolio.owner || "";
}

export function PortfolioSwitcher(props: { onCreate: () => void }) {
  const t = portfoliosText;
  const [open, setOpen] = createSignal(false);
  let root!: HTMLDivElement;
  let button!: HTMLButtonElement;
  let menu: HTMLDivElement | undefined;

  const current = currentPortfolio;
  const name = () => current()?.name ?? (noPortfolio() ? t().switcher.none : "…");
  /** Someone else's portfolio (an administrator looking at a member's book) shows its owner. */
  const owner = () => {
    const p = current();
    return p && ownerKind(p, me()?.username) === "other" ? p.owner_name || p.owner : null;
  };

  const items = () => Array.from(menu?.querySelectorAll<HTMLElement>("[data-item]") ?? []);
  const focusAt = (index: number) => {
    const list = items();
    if (list.length) list[(index + list.length) % list.length].focus();
  };

  const show = () => {
    setOpen(true);
    queueMicrotask(() => focusAt(Math.max(0, items().findIndex((el) => el.getAttribute("aria-checked") === "true"))));
  };
  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) button.focus();
  };

  const onDocument = (event: MouseEvent) => {
    if (open() && !root.contains(event.target as Node)) setOpen(false);
  };
  onMount(() => document.addEventListener("mousedown", onDocument));
  onCleanup(() => document.removeEventListener("mousedown", onDocument));

  const onMenuKey = (event: KeyboardEvent) => {
    const at = items().indexOf(document.activeElement as HTMLElement);
    const keys: Record<string, () => void> = {
      ArrowDown: () => focusAt(at + 1),
      ArrowUp: () => focusAt(at - 1),
      Home: () => focusAt(0),
      End: () => focusAt(-1),
      Escape: () => close(true),
    };
    if (event.key === "Tab") setOpen(false);
    const action = keys[event.key];
    if (!action) return;
    event.preventDefault();
    action();
  };

  const choose = (portfolio: Portfolio) => {
    close(true);
    if (portfolio.id !== current()?.id) selectPortfolio(portfolio.id);
  };

  return (
    <div class="pf-switch" ref={root}>
      <button
        ref={button}
        type="button"
        class="pf-switch-btn"
        aria-haspopup="menu"
        aria-expanded={open()}
        aria-label={tpl(t().switcher.button_label, { name: owner() ? `${name()} · ${owner()}` : name() })}
        title={owner() ? `${name()} · ${owner()}` : name()}
        onClick={() => (open() ? close(false) : show())}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            show();
          }
        }}
      >
        <Icon name="briefcase" size={15} />
        <span class="pf-switch-name">{name()}</span>
        <Show when={owner()}>
          <span class="pf-switch-owner">{owner()}</span>
        </Show>
        <Icon name="chevron_down" size={14} class="pf-switch-caret" />
      </button>
      <Show when={open()}>
        <div ref={menu} class="popover pf-menu" role="menu" aria-label={t().switcher.heading} onKeyDown={onMenuKey}>
          <div class="kicker pf-menu-head">{t().switcher.heading}</div>
          <Show when={portfoliosError() && !portfolios().length}>
            <div class="pf-menu-note">
              <span class="muted xs">{t().switcher.unavailable}</span>
              <button type="button" class="btn sm" data-item role="menuitem" onClick={() => void loadPortfolios()}>
                <Icon name="refresh" size={13} /> {t().switcher.retry}
              </button>
            </div>
          </Show>
          <div class="pf-menu-list">
            <For each={portfolios()}>
              {(p) => {
                const selected = () => p.id === current()?.id;
                return (
                  <button
                    type="button"
                    class="pf-item"
                    classList={{ current: selected() }}
                    role="menuitemradio"
                    aria-checked={selected()}
                    data-item
                    onClick={() => choose(p)}
                  >
                    <span class="pf-item-check" aria-hidden="true">
                      <Show when={selected()}>
                        <Icon name="check" size={14} />
                      </Show>
                    </span>
                    <span class="pf-item-main">
                      <span class="pf-item-top">
                        <span class="pf-item-name">{p.name}</span>
                        <ModeChip mode={p.effective_mode} />
                      </span>
                      <span class="pf-item-meta">
                        <span class="pf-item-owner">{ownerLabel(p)}</span>
                        <Show when={p.summary}>
                          {(s) => (
                            <>
                              <span class="num">{fmtMoney(s().nav, { compact: true })}</span>
                              <Pct value={s().total_return} />
                            </>
                          )}
                        </Show>
                      </span>
                    </span>
                  </button>
                );
              }}
            </For>
          </div>
          <div class="menu-sep" />
          <Show when={canCreate()}>
            <button
              type="button"
              class="menu-item"
              role="menuitem"
              data-item
              onClick={() => {
                close(false);
                props.onCreate();
              }}
            >
              <Icon name="plus" size={15} /> {t().actions.new_menu}
            </button>
          </Show>
          <A href="/portfolios" class="menu-item" role="menuitem" data-item onClick={() => close(false)}>
            <Icon name="briefcase" size={15} /> {t().actions.manage}
          </A>
        </div>
      </Show>
    </div>
  );
}
