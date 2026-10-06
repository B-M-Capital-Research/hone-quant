import { For, Show } from "solid-js";
import { Icon } from "@/components/Icon";
import { strategyText } from "@/i18n/strategy";
import type { Preset, StrategyParams } from "@/lib/types";
import { withDefaults } from "./params";
import { keyParams, pickText } from "./format";

/** Preset cards: bilingual name and summary, a few defining parameters, and a way to start from it. */
export function PresetCards(props: { presets: Preset[]; defaults: StrategyParams; activePresetId: string | null; onUse: (preset: Preset) => void }) {
  const t = strategyText;
  const pills = (preset: Preset) => {
    const all = keyParams(withDefaults(preset.params, props.defaults));
    // exposure, sector method, single-name cap, momentum tilt, turnover cap
    return [all[0], all[1], all[3], all[4], all[7]];
  };
  return (
    <div class="st-presets">
      <For each={props.presets}>
        {(preset) => (
          <article class="st-preset" classList={{ current: preset.id === props.activePresetId }}>
            <header>
              <h3>{pickText(preset, "name")}</h3>
              <Show when={preset.id === props.activePresetId}>
                <span class="chip green st-mini-chip">{t().presets.based}</span>
              </Show>
            </header>
            <p class="st-preset-summary">{pickText(preset, "summary")}</p>
            <dl class="st-preset-params">
              <For each={pills(preset)}>
                {(item) => (
                  <>
                    <dt>{item.label}</dt>
                    <dd>{item.value}</dd>
                  </>
                )}
              </For>
            </dl>
            <footer>
              <button class="btn sm" onClick={() => props.onUse(preset)}>
                <Icon name="sliders" size={13} /> {t().actions.use_as_start}
              </button>
            </footer>
          </article>
        )}
      </For>
    </div>
  );
}
