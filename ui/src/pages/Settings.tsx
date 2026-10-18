import { useContext, useEffect, useState } from "react";
import { api } from "../api";
import { ChampionsContext } from "../lib/assets";
import type { AiUsage, AppSettings, ProviderId, ProviderInfo, StatusView } from "../types";

interface Props {
  settings: AppSettings;
  status: StatusView | null;
  onSave: (s: AppSettings) => Promise<void>;
  onKeyChanged: () => void;
}

const SOURCE: Record<string, string> = {
  credential_manager: "saved in Windows Credential Manager",
  process_env: "RIOT_API_KEY environment variable",
  user_env: "RIOT_API_KEY user environment variable",
};

export function Settings({ settings, status, onSave, onKeyChanged }: Props) {
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const save = async (patch: (s: AppSettings) => AppSettings) => {
    setError(null);
    try {
      await onSave(patch(structuredClone(settings)));
      setSaved("Saved");
      window.setTimeout(() => setSaved(null), 1500);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="page settings">
      <div className="settings-head">
        <h1>Settings</h1>
        {saved && <span className="tag ok">{saved}</span>}
      </div>
      {error && <div className="banner error">{error}</div>}
      <RiotKey status={status} onChanged={onKeyChanged} />
      <AiSection settings={settings} save={save} />
      <DataSection settings={settings} save={save} status={status} />
      <Appearance settings={settings} save={save} />
    </div>
  );
}

type Save = (patch: (s: AppSettings) => AppSettings) => Promise<void>;

function RiotKey({ status, onChanged }: { status: StatusView | null; onChanged: () => void }) {
  const [key, setKey] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const run = async (f: () => Promise<unknown>, ok: string) => {
    setMsg(null);
    try {
      await f();
      setKey("");
      setMsg(ok);
      onChanged();
    } catch (e) {
      setMsg(String(e));
    }
  };
  const ks = status?.engine.key_status;
  return (
    <section className="panel">
      <h2>Riot API key</h2>
      <p>
        Status: <b className={`key-${ks}`}>{ks ?? "…"}</b>
        {status?.key_source && <span className="muted"> · {SOURCE[status.key_source]}</span>}
      </p>
      <p className="muted">
        A key saved here goes to Windows Credential Manager (never to a file) and takes priority over the
        RIOT_API_KEY environment variable. Personal keys don't expire; development keys expire every 24 hours.
      </p>
      <div className="form-row">
        <input
          className="grow"
          type="password"
          autoComplete="off"
          placeholder="RGAPI-…"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          aria-label="Riot API key"
        />
        <button disabled={!key.trim()} onClick={() => run(() => api.setRiotKey(key), "Key saved.")}>
          Save key
        </button>
        {status?.key_source === "credential_manager" && (
          <button className="secondary" onClick={() => run(() => api.deleteRiotKey(), "Saved key removed.")}>
            Remove saved key
          </button>
        )}
        <button className="secondary" onClick={() => run(() => api.reloadKey(), "Key reloaded.")}>
          Reload
        </button>
      </div>
      {msg && <p className="muted">{msg}</p>}
    </section>
  );
}

function AiSection({ settings, save }: { settings: AppSettings; save: Save }) {
  const ai = settings.ai;
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [key, setKey] = useState("");
  const [model, setModel] = useState(ai.model);
  const [baseUrl, setBaseUrl] = useState(ai.base_url ?? "");
  const [models, setModels] = useState<string[]>([]);
  const [msg, setMsg] = useState<{ tone: "ok" | "error"; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [prevAi, setPrevAi] = useState(ai);
  if (ai !== prevAi) {
    setPrevAi(ai);
    setModel(ai.model);
    setBaseUrl(ai.base_url ?? "");
  }

  const loadProviders = () => api.aiProviders().then(setProviders, () => {});
  useEffect(() => {
    api.aiProviders().then(setProviders, () => {});
  }, []);
  const info = providers.find((p) => p.id === ai.provider);
  const effectiveBase = baseUrl.trim() || null;

  const run = async (f: () => Promise<string | void>, ok: string) => {
    setBusy(true);
    setMsg(null);
    try {
      const r = await f();
      setMsg({ tone: "ok", text: r ? `${ok} ${r}` : ok });
    } catch (e) {
      setMsg({ tone: "error", text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="panel">
      <h2>AI coach</h2>
      <p className="muted">
        Reviews use your own API key with the provider you choose. The AI only ever sees the computed facts
        for a finished game (you can inspect them on any match), and runs only after games, never during one,
        per Riot's rules.
      </p>
      <div className="field">
        <label>Provider</label>
        <select
          value={ai.provider}
          onChange={(e) => {
            const p = providers.find((x) => x.id === e.target.value);
            setModels([]);
            void save((s) => ({
              ...s,
              ai: {
                ...s.ai,
                provider: e.target.value as ProviderId,
                model: p?.default_model ?? "",
                base_url: null,
              },
            }));
          }}
        >
          {providers.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
              {p.has_key ? "  ✓ key saved" : ""}
            </option>
          ))}
        </select>
      </div>
      <div className="field">
        <label>API key</label>
        <div className="form-row">
          <input
            className="grow"
            type="password"
            autoComplete="off"
            placeholder={
              info?.has_key
                ? "•••••• saved (enter a new one to replace)"
                : info?.key_required === false
                  ? "optional for local servers"
                  : "paste your key"
            }
            value={key}
            onChange={(e) => setKey(e.target.value)}
          />
          <button
            disabled={!key.trim() || busy}
            onClick={() =>
              run(async () => {
                await api.setAiKey(ai.provider, key);
                setKey("");
                await loadProviders();
              }, "Key saved to Windows Credential Manager.")
            }
          >
            Save key
          </button>
          {info?.has_key && (
            <button
              className="secondary"
              onClick={() =>
                run(async () => {
                  await api.deleteAiKey(ai.provider);
                  await loadProviders();
                }, "Key removed.")
              }
            >
              Remove
            </button>
          )}
        </div>
      </div>
      <div className="field">
        <label>Model</label>
        <div className="form-row">
          <input
            className="grow"
            list="ai-models"
            placeholder={info?.default_model || "pick a model (Load models lists them)"}
            value={model}
            onChange={(e) => setModel(e.target.value)}
            onBlur={() => model !== ai.model && save((s) => ({ ...s, ai: { ...s.ai, model: model.trim() } }))}
          />
          <datalist id="ai-models">
            {models.map((m) => (
              <option key={m} value={m} />
            ))}
          </datalist>
          <button
            className="secondary"
            disabled={busy}
            onClick={() =>
              run(async () => {
                const list = await api.listAiModels(ai.provider, effectiveBase);
                setModels(list);
                return `${list.length} models available; click the Model box to pick one.`;
              }, "")
            }
          >
            Load models
          </button>
        </div>
      </div>
      <div className="field">
        <label>Endpoint URL</label>
        <input
          placeholder={info?.default_base_url}
          value={baseUrl}
          onChange={(e) => setBaseUrl(e.target.value)}
          onBlur={() =>
            (effectiveBase ?? null) !== (ai.base_url ?? null) &&
            save((s) => ({ ...s, ai: { ...s.ai, base_url: effectiveBase } }))
          }
        />
        <span className="muted small-text">
          Leave empty for the default. For a model on your LAN (Ollama, LM Studio, vLLM), choose
          "OpenAI-compatible" and enter e.g. http://192.168.1.20:11434/v1.
        </span>
      </div>
      <div className="field">
        <label>Max output tokens</label>
        <input
          type="number"
          min={500}
          max={32000}
          step={500}
          value={ai.max_tokens}
          onChange={(e) => save((s) => ({ ...s, ai: { ...s.ai, max_tokens: Number(e.target.value) } }))}
        />
      </div>
      <Toggle
        label="Automatically review each new game"
        hint="When a game you just played finishes downloading, review it in the background. Never applies to older games pulled in by a sync."
        checked={ai.auto_post_game}
        onChange={(v) => save((s) => ({ ...s, ai: { ...s.ai, auto_post_game: v } }))}
      />
      <Toggle
        label="Automatically review each play session"
        hint="Once no game has been played for the session gap (Data & history), review the session that just ended. Only sessions with 2+ Summoner's Rift games from the last day."
        checked={ai.auto_session}
        onChange={(v) => save((s) => ({ ...s, ai: { ...s.ai, auto_session: v } }))}
      />
      <TokenCap settings={settings} save={save} />
      <p className="muted small-text">
        On-demand reviews are always available from any match and on the Sessions page. Weekly reports arrive
        in a later phase; every report is kept on the AI reports page.
      </p>
      <div className="form-row">
        <button
          className="secondary"
          disabled={busy || !model.trim()}
          onClick={() =>
            run(() => api.testAi(ai.provider, model.trim(), effectiveBase), "Connected. Model replied:")
          }
        >
          {busy ? "Working…" : "Test connection"}
        </button>
      </div>
      {msg && <div className={`banner ${msg.tone === "error" ? "error" : "ok"}`}>{msg.text}</div>}
    </section>
  );
}

function TokenCap({ settings, save }: { settings: AppSettings; save: Save }) {
  const limit = settings.ai.monthly_token_limit;
  const [usage, setUsage] = useState<AiUsage | null>(null);
  const [text, setText] = useState(limit === null ? "" : String(limit));
  const [prev, setPrev] = useState(limit);
  if (limit !== prev) {
    setPrev(limit);
    setText(limit === null ? "" : String(limit));
  }
  useEffect(() => {
    api.aiUsage().then(setUsage, () => {});
  }, [limit]);
  const commit = () => {
    const t = text.replace(/[^\d]/g, "");
    const next = t === "" ? null : Number(t);
    if (next !== limit) void save((s) => ({ ...s, ai: { ...s.ai, monthly_token_limit: next } }));
  };
  // Typical cost: a game review is about 5,000 tokens.
  return (
    <div className="field">
      <label>Token cap (last 30 days)</label>
      <input
        value={text}
        placeholder="No cap"
        inputMode="numeric"
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
      />
      <span className="muted small-text">
        {usage
          ? `Used ${usage.tokens_30d.toLocaleString()} tokens in ${usage.reports_30d} reports over the last 30 days. `
          : ""}
        A game review uses about 5,000 tokens. When the cap is reached, reviews stop (manual and automatic)
        until older ones age out. Leave empty for no cap.
      </span>
    </div>
  );
}

function DataSection({
  settings,
  save,
  status,
}: {
  settings: AppSettings;
  save: Save;
  status: StatusView | null;
}) {
  return (
    <section className="panel">
      <h2>Data & history</h2>
      <div className="field">
        <label>
          Look-back window: <b>{settings.baseline_games} games</b>
        </label>
        <input
          type="range"
          min={5}
          max={200}
          step={5}
          value={settings.baseline_games}
          onChange={(e) => save((s) => ({ ...s, baseline_games: Number(e.target.value) }))}
        />
        <span className="muted small-text">
          Your last N games used for baselines ("vs your games"), the dashboard, and trends.
        </span>
      </div>
      <div className="field">
        <label>Matches per sync / lookup</label>
        <input
          type="number"
          min={1}
          max={1000}
          value={settings.ingest_count}
          onChange={(e) => save((s) => ({ ...s, ingest_count: Number(e.target.value) }))}
        />
        <span className="muted small-text">
          About 100 games take 4–5 minutes with a personal key. Already stored games are skipped.
        </span>
      </div>
      <div className="field">
        <label>
          Session gap: <b>{settings.session_gap_min} minutes</b>
        </label>
        <input
          type="range"
          min={15}
          max={180}
          step={5}
          value={settings.session_gap_min}
          onChange={(e) => save((s) => ({ ...s, session_gap_min: Number(e.target.value) }))}
        />
        <span className="muted small-text">
          Games less than this far apart count as one play session (Sessions page, tilt patterns).
        </span>
      </div>
      <Toggle
        label="Show ARAM, Arena and other modes in the match list"
        hint="They're stored either way, but never count toward Summoner's Rift metrics."
        checked={settings.show_other_queues}
        onChange={(v) => save((s) => ({ ...s, show_other_queues: v }))}
      />
      <p className="muted small-text">
        Data folder: <span className="mono">{status?.data_dir}</span>. Everything (matches, reviews, settings,
        history) is saved there and survives restarts. Rift Coach {status?.version}.
      </p>
    </section>
  );
}

function Appearance({ settings, save }: { settings: AppSettings; save: Save }) {
  const champs = useContext(ChampionsContext);
  const a = settings.appearance;
  const list = [...champs.values()].sort((x, y) => x.name.localeCompare(y.name));
  return (
    <section className="panel">
      <h2>Appearance</h2>
      <Toggle
        label="Animated champion background"
        hint="Splash art of the match you're viewing, otherwise your most-played champion."
        checked={a.animated_background}
        onChange={(v) => save((s) => ({ ...s, appearance: { ...s.appearance, animated_background: v } }))}
      />
      <Toggle
        label="Floating particles"
        checked={a.particles}
        onChange={(v) => save((s) => ({ ...s, appearance: { ...s.appearance, particles: v } }))}
      />
      <div className="field">
        <label>Intensity</label>
        <input
          type="range"
          min={0}
          max={1}
          step={0.05}
          value={a.background_intensity}
          onChange={(e) =>
            save((s) => ({
              ...s,
              appearance: { ...s.appearance, background_intensity: Number(e.target.value) },
            }))
          }
        />
      </div>
      <div className="field">
        <label>Champion</label>
        <select
          value={a.background_champion ?? ""}
          onChange={(e) =>
            save((s) => ({
              ...s,
              appearance: { ...s.appearance, background_champion: e.target.value || null },
            }))
          }
        >
          <option value="">Automatic (open match, else most played)</option>
          {list.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
      </div>
      <p className="muted small-text">Animations pause automatically when Windows "reduce motion" is on.</p>
    </section>
  );
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="toggle">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span className="switch" />
      <span>
        {label}
        {hint && <span className="muted small-text block">{hint}</span>}
      </span>
    </label>
  );
}
