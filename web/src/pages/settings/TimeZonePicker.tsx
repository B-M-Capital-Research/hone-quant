import { For, Show, createEffect, createSignal, on } from "solid-js";
import { settingsText } from "@/i18n/settings";
import { serverNow } from "@/lib/session";
import { canonicalTimeZone, utcOffsetLabel } from "./time";

export const COMMON_ZONES = ["Asia/Singapore", "Asia/Shanghai", "Asia/Hong_Kong", "America/New_York", "UTC"] as const;
const OTHER = "__other__";

const isCommon = (tz: string) => (COMMON_ZONES as readonly string[]).includes(tz);

export function zoneName(tz: string): string {
  const names = settingsText().tz as Record<string, string>;
  return isCommon(tz) ? names[tz] : tz;
}

/** Validation for a free-entry zone, mirroring the server's IANA lookup. */
export function timeZoneError(value: string): string | undefined {
  return canonicalTimeZone(value) ? undefined : settingsText().v.timezone;
}

/** Common zones in a select, plus free entry of any IANA name (validated by the browser). */
export function TimeZonePicker(props: {
  id: string;
  value: string;
  onChange: (tz: string) => void;
  onBlur?: () => void;
  disabled?: boolean;
  invalid?: boolean;
}) {
  const t = settingsText;
  const [custom, setCustom] = createSignal(!isCommon(props.value));
  createEffect(
    on(
      () => props.value,
      (v) => {
        if (isCommon(v)) setCustom(false);
        else if (v) setCustom(true);
      },
      { defer: true },
    ),
  );
  const label = (tz: string) => {
    const offset = utcOffsetLabel(tz, serverNow());
    return tz === "UTC" ? zoneName(tz) : `${zoneName(tz)} · ${tz}${offset ? ` (${offset})` : ""}`;
  };
  return (
    <div class="tz-picker">
      <select
        id={props.id}
        class="select"
        value={custom() ? OTHER : props.value}
        disabled={props.disabled}
        aria-invalid={props.invalid && !custom() ? "true" : "false"}
        onChange={(e) => {
          const v = e.currentTarget.value;
          if (v === OTHER) setCustom(true);
          else {
            setCustom(false);
            props.onChange(v);
          }
        }}
      >
        <For each={COMMON_ZONES}>{(tz) => <option value={tz}>{label(tz)}</option>}</For>
        <option value={OTHER}>{t().tz.other}</option>
      </select>
      <Show when={custom()}>
        <input
          id={`${props.id}-custom`}
          class="input mono"
          classList={{ invalid: !!props.invalid }}
          aria-invalid={props.invalid ? "true" : "false"}
          aria-label={t().tz.custom_label}
          placeholder={t().tz.custom_placeholder}
          autocomplete="off"
          spellcheck={false}
          value={props.value}
          disabled={props.disabled}
          onInput={(e) => props.onChange(e.currentTarget.value)}
          onBlur={(e) => {
            const canonical = canonicalTimeZone(e.currentTarget.value);
            if (canonical && canonical !== e.currentTarget.value) props.onChange(canonical);
            props.onBlur?.();
          }}
        />
      </Show>
    </div>
  );
}
