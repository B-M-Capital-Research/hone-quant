/**
 * Renaming a portfolio and archiving it. Archiving is destructive (it cancels pending plans and
 * stops trading), so it asks for the word ARCHIVE and only the explicit button submits it.
 */
import { For, Show, createMemo, createSignal, onMount } from "solid-js";
import { Icon } from "@/components/Icon";
import { Dialog, toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { portfoliosText } from "@/i18n/portfolios";
import { ApiError, api } from "@/lib/api";
import { upsertPortfolio } from "@/lib/portfolio";
import type { Portfolio } from "@/lib/types";
import "@/styles/portfolios.css";
import { focusInvalid, nameProblem } from "./form";

export function EditPortfolioDialog(props: { portfolio: Portfolio; onClose: () => void; onSaved: (portfolio: Portfolio) => void }) {
  const t = portfoliosText;
  const c = common;
  const [name, setName] = createSignal(props.portfolio.name);
  const [description, setDescription] = createSignal(props.portfolio.description);
  const [touched, setTouched] = createSignal(false);
  const [serverError, setServerError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  let nameInput: HTMLInputElement | undefined;
  onMount(() => requestAnimationFrame(() => nameInput?.focus()));

  const nameError = createMemo(() => {
    if (serverError()) return serverError()!;
    if (!touched()) return undefined;
    const problem = nameProblem(name());
    return problem === "required" ? t().create.err_name_required : problem === "too_long" ? t().create.err_name_long : undefined;
  });

  const submit = async () => {
    setTouched(true);
    if (nameProblem(name())) {
      focusInvalid();
      return;
    }
    setBusy(true);
    try {
      const { portfolio } = await api.updatePortfolio(props.portfolio.id, { name: name().trim(), description: description().trim() });
      toast(t().edit.saved, portfolio.name, "success");
      upsertPortfolio(portfolio);
      props.onSaved(portfolio);
    } catch (error) {
      if (error instanceof ApiError && (error.status === 409 || (error.status === 400 && /name/i.test(error.message)))) {
        setServerError(error.status === 409 ? t().create.err_name_taken : error.message);
        focusInvalid();
      } else {
        toastError(error);
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={t().edit.title}
      onClose={() => !busy() && props.onClose()}
      footer={
        <>
          <button type="button" class="btn" onClick={() => props.onClose()} disabled={busy()}>
            {c().actions.cancel}
          </button>
          <button type="button" class="btn primary" onClick={() => void submit()} disabled={busy()}>
            {busy() ? c().actions.saving : t().edit.submit}
          </button>
        </>
      }
    >
      <form
        class="stack pf-form"
        style={{ gap: "16px" }}
        novalidate
        autocomplete="off"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <div class="field">
          <label for="pf-edit-name">{t().create.name}</label>
          <input
            ref={nameInput}
            id="pf-edit-name"
            class="input"
            classList={{ invalid: !!nameError() }}
            aria-invalid={nameError() ? "true" : "false"}
            value={name()}
            onInput={(e) => {
              setName(e.currentTarget.value);
              setServerError(null);
            }}
            onBlur={() => setTouched(true)}
          />
          <Show when={nameError()} fallback={<span class="hint">{t().create.name_hint}</span>}>
            <span class="error-text" role="alert">
              {nameError()}
            </span>
          </Show>
        </div>
        <div class="field">
          <label for="pf-edit-description">{t().create.description}</label>
          <textarea
            id="pf-edit-description"
            class="textarea"
            rows={3}
            maxLength={500}
            placeholder={t().create.description_placeholder}
            value={description()}
            onInput={(e) => setDescription(e.currentTarget.value)}
          />
        </div>
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}

export function ArchivePortfolioDialog(props: { portfolio: Portfolio; onClose: () => void; onArchived: (portfolio: Portfolio) => void }) {
  const t = portfoliosText;
  const c = common;
  const [reason, setReason] = createSignal("");
  const [confirmText, setConfirmText] = createSignal("");
  const [problem, setProblem] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const confirmed = () => confirmText().trim() === "ARCHIVE";
  const confirmError = () => (confirmText().trim().length >= 7 && !confirmed() ? t().archive.confirm_mismatch : undefined);

  const submit = async () => {
    if (!confirmed()) return;
    setBusy(true);
    setProblem(null);
    try {
      const { portfolio } = await api.archivePortfolio(props.portfolio.id, reason().trim());
      toast(tpl(t().archive.done, { name: props.portfolio.name }), undefined, "success");
      props.onArchived(portfolio);
      // Last: when this was the current portfolio the app switches away (and remounts the page).
      upsertPortfolio(portfolio);
    } catch (error) {
      if (error instanceof ApiError && error.status === 409) setProblem(t().archive.executing);
      else if (error instanceof ApiError && error.status === 400) setProblem(error.message);
      else toastError(error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={t().archive.title}
      subtitle={props.portfolio.name}
      onClose={() => !busy() && props.onClose()}
      footer={
        <>
          <button type="button" class="btn" onClick={() => props.onClose()} disabled={busy()}>
            {c().actions.cancel}
          </button>
          <button type="button" class="btn danger solid" disabled={busy() || !confirmed()} onClick={() => void submit()}>
            {busy() ? t().archive.archiving : t().archive.submit}
          </button>
        </>
      }
    >
      {/* Destructive: Enter never submits, only the explicit button does. */}
      <form class="stack pf-form" style={{ gap: "16px" }} novalidate onSubmit={(e) => e.preventDefault()}>
        <div class="callout critical">
          <Icon name="alert" size={16} />
          <span>{tpl(t().archive.warning, { name: props.portfolio.name })}</span>
        </div>
        <ul class="pf-points">
          <For each={t().archive.points}>{(point) => <li>{point}</li>}</For>
        </ul>
        <div class="field">
          <label for="pf-archive-reason">{t().archive.reason}</label>
          <textarea
            id="pf-archive-reason"
            class="textarea"
            rows={2}
            maxLength={500}
            placeholder={t().archive.reason_placeholder}
            value={reason()}
            onInput={(e) => setReason(e.currentTarget.value)}
          />
        </div>
        <div class="field">
          <label for="pf-archive-confirm">{t().archive.confirm_label}</label>
          <input
            id="pf-archive-confirm"
            class="input mono"
            classList={{ invalid: !!confirmError() }}
            aria-invalid={confirmError() ? "true" : "false"}
            autocomplete="off"
            autocapitalize="characters"
            spellcheck={false}
            placeholder="ARCHIVE"
            value={confirmText()}
            onInput={(e) => {
              setConfirmText(e.currentTarget.value);
              setProblem(null);
            }}
          />
          <Show when={confirmError()}>
            <span class="error-text" role="alert">
              {confirmError()}
            </span>
          </Show>
        </div>
        <Show when={problem()}>
          <div class="callout critical" role="alert">
            <Icon name="alert" size={16} />
            <span>{problem()}</span>
          </div>
        </Show>
        <p class="muted xs">{c().confirm.irreversible}</p>
      </form>
    </Dialog>
  );
}
