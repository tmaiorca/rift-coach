import { useContext, useEffect, useState } from "react";
import { api } from "../api";
import { ChampionsContext } from "../lib/assets";
import type {
  AiUsage,
  AppSettings,
  McpStatus,
  ProviderId,
  ProviderInfo,
  StatusView,
  UpdateInfo,
} from "../types";

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
      <LiveSection settings={settings} save={save} />
      <BenchmarkSection settings={settings} save={save} />
      <DesktopSection settings={settings} save={save} />
      <McpSection settings={settings} save={save} />
      <DataSection settings={settings} save={save} status={status} />
      <DataTools />
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
      <Toggle
        label="Automatically write a weekly report"
        hint="After each week (Monday to Sunday) with 3+ Summoner's Rift games, a report on the week's trends. Checked every 10 minutes."
        checked={ai.auto_weekly}
        onChange={(v) => save((s) => ({ ...s, ai: { ...s.ai, auto_weekly: v } }))}
      />
      <Toggle
        label="Write an AI game plan at the loading screen"
        hint="A short, static plan from the champions in your game and your own history. Written once on the loading screen, never updated during the game."
        checked={ai.auto_pregame}
        onChange={(v) => save((s) => ({ ...s, ai: { ...s.ai, auto_pregame: v } }))}
      />
      <TokenCap settings={settings} save={save} />
      <p className="muted small-text">
        On-demand reviews are always available from any match and on the Sessions and Weekly pages, and a game
        plan can be requested on the loading screen. Every report is kept on the AI reports page.
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

const TIERS: { id: AppSettings["benchmarks"]["tier"]; label: string }[] = [
  { id: "AUTO", label: "Auto: two tiers above your solo rank" },
  { id: "GOLD", label: "Gold" },
  { id: "PLATINUM", label: "Platinum" },
  { id: "EMERALD", label: "Emerald" },
  { id: "DIAMOND", label: "Diamond" },
  { id: "MASTER", label: "Master and above" },
];

function BenchmarkSection({ settings, save }: { settings: AppSettings; save: Save }) {
  const champs = useContext(ChampionsContext);
  const list = [...champs.values()].sort((x, y) => x.name.localeCompare(y.name));
  const b = settings.benchmarks;
  const setB = (patch: Partial<AppSettings["benchmarks"]>) =>
    save((s) => ({ ...s, benchmarks: { ...s.benchmarks, ...patch } }));
  return (
    <section className="panel">
      <h2>Benchmarks</h2>
      <Toggle
        label="Collect higher-rank games in the background"
        hint="Never during a game or while your own games download. Uses well under half of your Riot key's rate limit."
        checked={b.enabled}
        onChange={(v) => setB({ enabled: v })}
      />
      <div className="field">
        <label>Compare with</label>
        <select value={b.tier} onChange={(e) => setB({ tier: e.target.value as typeof b.tier })}>
          {TIERS.map((t) => (
            <option key={t.id} value={t.id}>
              {t.label}
            </option>
          ))}
        </select>
        <span className="muted small-text">
          Changing it starts a new sample; games already collected for the old tier stay until they age out.
        </span>
      </div>
      <div className="field">
        <label>Champions to sample first</label>
        <div className="form-row">
          <select
            value=""
            onChange={(e) =>
              e.target.value && setB({ champions: [...b.champions, e.target.value].slice(0, 3) })
            }
            disabled={b.champions.length >= 3}
          >
            <option value="">{b.champions.length >= 3 ? "Up to 3" : "Add a champion…"}</option>
            {list
              .filter((c) => !b.champions.includes(c.id))
              .map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
          </select>
          {b.champions.map((c) => (
            <button
              key={c}
              className="secondary small"
              onClick={() => setB({ champions: b.champions.filter((x) => x !== c) })}
            >
              {c} ✕
            </button>
          ))}
        </div>
        <span className="muted small-text">Empty = your most played (5+ games in your last 50).</span>
      </div>
      <div className="field">
        <label>
          Pace: <b>one step every {b.step_seconds} s</b>
        </label>
        <input
          type="range"
          min={2}
          max={30}
          step={1}
          value={b.step_seconds}
          onChange={(e) => setB({ step_seconds: Number(e.target.value) })}
        />
        <span className="muted small-text">
          Each step makes 1-2 Riot calls. Slower leaves more room for other tools using your key.
        </span>
      </div>
      <div className="field">
        <label>
          Keep up to <b>{b.max_games.toLocaleString()} games</b> (about{" "}
          {Math.round((b.max_games * 60) / 1024)} MB)
        </label>
        <input
          type="range"
          min={500}
          max={10000}
          step={500}
          value={b.max_games}
          onChange={(e) => setB({ max_games: Number(e.target.value) })}
        />
        <span className="muted small-text">Only the two newest patches are kept either way.</span>
      </div>
    </section>
  );
}

function LiveSection({ settings, save }: { settings: AppSettings; save: Save }) {
  const l = settings.live;
  const [target, setTarget] = useState(l.cs_target_pm?.toString() ?? "");
  const setLive = (patch: Partial<AppSettings["live"]>) =>
    save((s) => ({ ...s, live: { ...s.live, ...patch } }));
  return (
    <section className="panel">
      <h2>Live</h2>
      <p className="muted">
        While a game runs, Rift Coach reads only your own numbers from the game client (127.0.0.1:2999) and
        shows facts plus targets you set beforehand. No AI, advice, alerts, or other players' information
        during games, per Riot's rules.
      </p>
      <Toggle
        label="Detect running games"
        hint="Checks every few seconds (backing off to 15 s when nothing runs) for the live view and post-game analysis."
        checked={l.enabled}
        onChange={(v) => setLive({ enabled: v })}
      />
      <Toggle
        label="Pause background work during games"
        hint="Match fetching, analysis, and AI reviews wait until the game ends, so the game gets your PC."
        checked={l.pause_background}
        onChange={(v) => setLive({ pause_background: v })}
      />
      <Toggle
        label="Notify me when a finished game is analyzed"
        hint="A Windows notification once Riot publishes the game and it's analyzed (or its AI review is ready)."
        checked={l.notify}
        onChange={(v) => setLive({ notify: v })}
      />
      <div className="field">
        <label>CS per minute target</label>
        <div className="form-row">
          <input
            type="number"
            min={1}
            max={15}
            step={0.1}
            placeholder="Automatic"
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            onBlur={() => {
              const v = Number(target);
              void setLive({ cs_target_pm: target.trim() && v > 0 ? v : null });
            }}
          />
        </div>
        <span className="muted small-text">
          Empty = an active CS/min focus goal if you have one, else your own average in that role.
        </span>
      </div>
      <div className="field">
        <label>
          Keep live snapshots:{" "}
          <b>{l.keep_snapshots_days === 0 ? "delete once analyzed" : `${l.keep_snapshots_days} days`}</b>
        </label>
        <input
          type="range"
          min={0}
          max={60}
          step={1}
          value={l.keep_snapshots_days}
          onChange={(e) => setLive({ keep_snapshots_days: Number(e.target.value) })}
        />
        <span className="muted small-text">
          Your numbers every 15 s of game time. The game's record and footprint stay either way.
        </span>
      </div>
    </section>
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

function DesktopSection({ settings, save }: { settings: AppSettings; save: Save }) {
  const d = settings.desktop;
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [update, setUpdate] = useState<UpdateInfo | null | undefined>(undefined);
  const [msg, setMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.autostartEnabled().then(setAutostart, () => setAutostart(false));
  }, []);
  const check = async () => {
    setBusy(true);
    setMsg(null);
    try {
      setUpdate(await api.checkUpdate());
    } catch (e) {
      setMsg(`Couldn't check for updates: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };
  const install = async () => {
    setBusy(true);
    setMsg("Downloading the update… Rift Coach restarts when it's installed.");
    try {
      await api.installUpdate();
    } catch (e) {
      setMsg(String(e));
      setBusy(false);
    }
  };
  return (
    <section className="panel">
      <h2>Desktop</h2>
      <Toggle
        label="Start with Windows"
        hint="Starts in the tray when you sign in, so your games are tracked without opening anything."
        checked={autostart ?? false}
        onChange={(v) => api.setAutostart(v).then(setAutostart, (e) => setMsg(String(e)))}
      />
      <Toggle
        label="Keep running in the tray when the window is closed"
        hint="Needed to catch games and write post-game analysis while the window is closed. Quit from the tray icon."
        checked={d.close_to_tray}
        onChange={(v) => save((s) => ({ ...s, desktop: { ...s.desktop, close_to_tray: v } }))}
      />
      <Toggle
        label="Check for updates on start"
        hint="Only signed Rift Coach releases are installed, and only when you click Install."
        checked={d.check_updates}
        onChange={(v) => save((s) => ({ ...s, desktop: { ...s.desktop, check_updates: v } }))}
      />
      <div className="form-row">
        <button className="secondary" disabled={busy} onClick={check}>
          Check for updates now
        </button>
        {update && (
          <button disabled={busy} onClick={install}>
            Install {update.version} and restart
          </button>
        )}
        {update === null && <span className="muted">You're on the latest version.</span>}
      </div>
      {update?.notes && <p className="muted small-text pre-wrap">{update.notes}</p>}
      {msg && <p className="muted">{msg}</p>}
    </section>
  );
}

function McpSection({ settings, save }: { settings: AppSettings; save: Save }) {
  const m = settings.mcp;
  const [st, setSt] = useState<McpStatus | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  useEffect(() => {
    // The server restarts after a save; give it a moment.
    const id = window.setTimeout(() => api.mcpStatus().then(setSt, () => {}), 300);
    return () => window.clearTimeout(id);
  }, [m]);
  const setM = (patch: Partial<AppSettings["mcp"]>) => save((s) => ({ ...s, mcp: { ...s.mcp, ...patch } }));
  const copy = async (text: string, what: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setMsg(`${what} copied.`);
    } catch (e) {
      setMsg(String(e));
    }
  };
  const claudeConfig = st
    ? JSON.stringify(
        { mcpServers: { "rift-coach": { command: st.exe_path, args: ["--mcp-stdio"] } } },
        null,
        2,
      )
    : "";
  return (
    <section className="panel">
      <h2>AI agents (MCP)</h2>
      <p className="muted">
        Lets Claude Desktop or your own AI agent read your Rift Coach data: matches, metrics, patterns, AI
        reviews, goals, weekly summaries and benchmarks. Read-only: agents can't change anything, and other
        players' identities are never included.
      </p>
      <h3>Claude Desktop (and other apps that launch a command)</h3>
      <p className="muted small-text">
        Works without the server below. In Claude Desktop open Settings → Developer → Edit Config, add this,
        and restart Claude Desktop.
      </p>
      <pre className="facts">{claudeConfig}</pre>
      <button className="secondary small" disabled={!st} onClick={() => copy(claudeConfig, "Config")}>
        Copy config
      </button>
      <h3>HTTP server (agents that connect to a URL)</h3>
      <Toggle
        label="Run the MCP server"
        hint={`Streamable HTTP on port ${m.port}, on this PC only unless you allow your network below.`}
        checked={m.enabled}
        onChange={(v) => setM({ enabled: v })}
      />
      <Toggle
        label="Allow devices on my network"
        hint="For an agent on another machine (e.g. a home server). Always requires the access token. Windows may ask to let Rift Coach through the firewall."
        checked={m.lan}
        onChange={(v) => setM({ lan: v })}
      />
      <Toggle
        label="Require the token on this PC too"
        checked={m.require_token_local}
        onChange={(v) => setM({ require_token_local: v })}
      />
      <div className="field">
        <label>Port</label>
        <input
          type="number"
          min={1024}
          max={65535}
          value={m.port}
          onChange={(e) => setM({ port: Number(e.target.value) })}
        />
      </div>
      {st && m.enabled && (
        <div className="mcp-status">
          {st.running ? (
            <>
              <p>
                <span className="tag ok">Running</span> <span className="mono">{st.local_url}</span>
                {st.lan_url && (
                  <>
                    {" "}
                    · on your network: <span className="mono">{st.lan_url}</span>
                  </>
                )}
              </p>
              {st.token_required && (
                <p className="muted small-text">
                  Agents send the token as the header{" "}
                  <span className="mono">Authorization: Bearer &lt;token&gt;</span>.
                </p>
              )}
            </>
          ) : (
            <p className="bad">Not running{st.error ? `: ${st.error}` : ""}</p>
          )}
        </div>
      )}
      <div className="form-row">
        <button
          className="secondary small"
          onClick={() =>
            api.mcpToken().then(
              (t) => copy(t, "Access token"),
              (e) => setMsg(String(e)),
            )
          }
        >
          Copy access token
        </button>
        <button
          className="secondary small"
          onClick={() =>
            window.confirm(
              "Make a new token? Agents using the current one stop working until you give them the new one.",
            ) &&
            api.mcpNewToken().then(
              () => setMsg("New token made."),
              (e) => setMsg(String(e)),
            )
          }
        >
          New token
        </button>
      </div>
      {msg && <p className="muted">{msg}</p>}
    </section>
  );
}

function DataTools() {
  const [msg, setMsg] = useState<string | null>(null);
  const [forget, setForget] = useState(false);
  const run = async (f: () => Promise<string | null>, done: (r: string) => string) => {
    setMsg(null);
    try {
      const r = await f();
      if (r) setMsg(done(r));
    } catch (e) {
      setMsg(String(e));
    }
  };
  const restore = async () => {
    const info = await api.restoreDatabase();
    return info ? `Restoring ${info.matches} matches… restarting.` : null;
  };
  return (
    <section className="panel">
      <h2>Your data</h2>
      <p className="muted">
        Everything Rift Coach knows lives in one database on this PC. Back it up, take your games to a
        spreadsheet, or start over.
      </p>
      <div className="form-row">
        <button className="secondary" onClick={() => run(api.backupDatabase, (p) => `Backup saved: ${p}`)}>
          Back up…
        </button>
        <button
          className="secondary"
          onClick={() =>
            window.confirm(
              "Restore a backup? Your current data is replaced (a copy is kept in the data folder as rift-coach.before-restore.db) and Rift Coach restarts.",
            ) && run(restore, (m) => m)
          }
        >
          Restore…
        </button>
        <button className="secondary" onClick={() => run(api.exportGamesCsv, (p) => `Exported: ${p}`)}>
          Export games (CSV)…
        </button>
        <button
          className="secondary"
          onClick={() => void api.openDataFolder().catch((e) => setMsg(String(e)))}
        >
          Open data folder
        </button>
      </div>
      <h3>Delete all my data</h3>
      <p className="muted small-text">
        Removes every game, review, goal, benchmark, setting and log from this PC, then restarts as a fresh
        install. This can't be undone: back up first if you might want it back.
      </p>
      <label className="toggle">
        <input type="checkbox" checked={forget} onChange={(e) => setForget(e.target.checked)} />
        <span className="switch" />
        <span>Also remove saved keys (Riot, AI, MCP token) and turn off start with Windows</span>
      </label>
      <div className="form-row">
        <button
          className="secondary danger"
          onClick={() =>
            window.confirm("Delete ALL Rift Coach data on this PC? This can't be undone.") &&
            void api.deleteAllData(forget).catch((e) => setMsg(String(e)))
          }
        >
          Delete all my data
        </button>
      </div>
      {msg && <p className="muted">{msg}</p>}
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

export function Toggle({
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
