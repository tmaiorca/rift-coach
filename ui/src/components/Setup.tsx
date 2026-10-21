// First-run setup: Riot key, your Riot ID, an optional AI provider, and
// desktop preferences. Every step can be skipped and redone in Settings.
import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useState } from "react";
import { api } from "../api";
import { PLATFORMS } from "../lib/format";
import type { AppSettings, ProviderId, ProviderInfo, StatusView } from "../types";

const STEPS = ["Welcome", "Riot API key", "Your account", "AI coach", "Desktop", "Done"] as const;

const TIERS: { id: AppSettings["benchmarks"]["tier"]; label: string }[] = [
  { id: "AUTO", label: "Two tiers above my rank (recommended)" },
  { id: "GOLD", label: "Gold" },
  { id: "PLATINUM", label: "Platinum" },
  { id: "EMERALD", label: "Emerald" },
  { id: "DIAMOND", label: "Diamond" },
  { id: "MASTER", label: "Master and above" },
];

export function Setup({
  settings,
  status,
  onSave,
  onKeyChanged,
  onAccountAdded,
  onDone,
}: {
  settings: AppSettings;
  status: StatusView | null;
  onSave: (s: AppSettings) => Promise<void>;
  onKeyChanged: () => void;
  onAccountAdded: () => void;
  onDone: () => void;
}) {
  const [step, setStep] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Riot key
  const [riotKey, setRiotKey] = useState("");
  const keyStatus = status?.engine.key_status;
  const hasKey = !!status?.key_source;
  // Account
  const [riotId, setRiotId] = useState("");
  const [platform, setPlatform] = useState("na1");
  const [added, setAdded] = useState<string | null>(
    status?.account ? `${status.account.game_name}#${status.account.tag_line}` : null,
  );
  // AI
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [provider, setProvider] = useState<ProviderId>(settings.ai.provider);
  const [aiKey, setAiKey] = useState("");
  const [aiOk, setAiOk] = useState<string | null>(null);
  // Desktop
  const [autostart, setAutostart] = useState(false);

  useEffect(() => {
    api.aiProviders().then(setProviders, () => {});
    api.autostartEnabled().then(setAutostart, () => {});
  }, []);

  const act = async (f: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const next = () => (setError(null), setStep((s) => Math.min(s + 1, STEPS.length - 1)));
  const back = () => (setError(null), setStep((s) => Math.max(s - 1, 0)));
  const info = providers.find((p) => p.id === provider);

  const finish = () =>
    act(async () => {
      await onSave({ ...settings, desktop: { ...settings.desktop, onboarded: true } });
      onDone();
    });

  return (
    <div className="setup">
      <div className="setup-card panel">
        <div className="setup-steps">
          {STEPS.map((s, i) => (
            <span key={s} className={`setup-step ${i === step ? "on" : i < step ? "done" : ""}`}>
              {s}
            </span>
          ))}
        </div>

        {step === 0 && (
          <>
            <h1>
              <span className="brand-mark">⬡</span> Welcome to Rift Coach
            </h1>
            <p>
              Rift Coach downloads your League of Legends games, measures what happened in each one, finds the
              habits that win and lose you games, and compares you with higher-ranked players on the champions
              you actually play.
            </p>
            <ul className="muted">
              <li>Everything stays on this PC. Keys go to Windows Credential Manager, never to a file.</li>
              <li>
                During a game it only shows your own numbers. Nothing reads game memory or reacts to the game
                while you play, so it stays within Riot's rules.
              </li>
              <li>Setup takes about two minutes. You can skip any step and finish it in Settings.</li>
            </ul>
          </>
        )}

        {step === 1 && (
          <>
            <h2>Riot API key</h2>
            <p>
              Rift Coach reads your match history from Riot's API, which needs a key of your own (it's free).
            </p>
            <ol className="muted">
              <li>
                Sign in at{" "}
                <button className="link" onClick={() => void openUrl("https://developer.riotgames.com/")}>
                  developer.riotgames.com
                </button>{" "}
                with your Riot account.
              </li>
              <li>
                For a quick start, copy the <b>Development API key</b> from the dashboard (it expires every 24
                hours). For a key that lasts, register a <b>Personal</b> app there and use its key.
              </li>
              <li>Paste it below. It starts with RGAPI-.</li>
            </ol>
            {hasKey && (
              <p>
                <span className={`tag ${keyStatus === "rejected" ? "warn" : "ok"}`}>
                  {keyStatus === "rejected" ? "Key rejected" : "Key found"}
                </span>{" "}
                {keyStatus === "rejected"
                  ? "Riot refused the current key (development keys expire daily). Paste a new one."
                  : "You can keep it, or paste a different one."}
              </p>
            )}
            <div className="form-row">
              <input
                className="grow"
                type="password"
                autoComplete="off"
                placeholder="RGAPI-…"
                value={riotKey}
                onChange={(e) => setRiotKey(e.target.value)}
                aria-label="Riot API key"
              />
              <button
                disabled={!riotKey.trim() || busy}
                onClick={() =>
                  act(async () => {
                    await api.setRiotKey(riotKey);
                    setRiotKey("");
                    onKeyChanged();
                    next();
                  })
                }
              >
                Save key
              </button>
            </div>
          </>
        )}

        {step === 2 && (
          <>
            <h2>Your account</h2>
            <p>
              Enter your Riot ID as it shows in the client (name and tag) and your server. Your newest{" "}
              {settings.ingest_count} games download in the background; about 100 take 4–5 minutes.
            </p>
            {added && (
              <p>
                <span className="tag ok">Added</span> {added}. You can add more accounts (smurfs, a friend on
                this PC) on the Players page later.
              </p>
            )}
            <div className="form-row">
              <input
                className="grow"
                placeholder="Name#TAG"
                value={riotId}
                onChange={(e) => setRiotId(e.target.value)}
                aria-label="Riot ID"
              />
              <select value={platform} onChange={(e) => setPlatform(e.target.value)} aria-label="Server">
                {PLATFORMS.map((p) => (
                  <option key={p} value={p}>
                    {p.toUpperCase()}
                  </option>
                ))}
              </select>
              <button
                disabled={!riotId.includes("#") || busy || !hasKey}
                onClick={() =>
                  act(async () => {
                    await api.startIngest(riotId.trim(), platform, settings.ingest_count);
                    setAdded(riotId.trim());
                    setRiotId("");
                    onAccountAdded();
                  })
                }
              >
                Add and download games
              </button>
            </div>
            {!hasKey && <p className="muted small-text">Add a Riot API key first (previous step).</p>}
            <p className="muted small-text">
              Coaching follows the champions and roles you play most: patterns, champion pages and higher-rank
              benchmarks pick them up automatically from your games.
            </p>
          </>
        )}

        {step === 3 && (
          <>
            <h2>AI coach (optional)</h2>
            <p>
              Rift Coach can ask an AI model to turn your numbers into a short review with the top things to
              fix. The AI only sees your stats and patterns, never other players' names. You pay your AI
              provider directly; without a key everything else still works.
            </p>
            <div className="form-row">
              <select
                value={provider}
                onChange={(e) => (setProvider(e.target.value as ProviderId), setAiOk(null))}
                aria-label="AI provider"
              >
                {providers.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label}
                  </option>
                ))}
              </select>
              {info?.key_required !== false && (
                <input
                  className="grow"
                  type="password"
                  autoComplete="off"
                  placeholder={info?.has_key ? "Key saved (paste to replace)" : "API key"}
                  value={aiKey}
                  onChange={(e) => setAiKey(e.target.value)}
                  aria-label="AI API key"
                />
              )}
              <button
                disabled={busy || (!aiKey.trim() && !info?.has_key && info?.key_required !== false)}
                onClick={() =>
                  act(async () => {
                    if (aiKey.trim()) await api.setAiKey(provider, aiKey);
                    setAiKey("");
                    const model = info?.default_model ?? settings.ai.model;
                    await onSave({ ...settings, ai: { ...settings.ai, provider, model } });
                    const reply = await api.testAi(provider, model, settings.ai.base_url);
                    setAiOk(`Working (${model} replied "${reply.trim()}").`);
                    void api.aiProviders().then(setProviders, () => {});
                  })
                }
              >
                Save and test
              </button>
            </div>
            {aiOk && <p className="good">{aiOk}</p>}
            <p className="muted small-text">
              A model on your own server (OpenAI-compatible) and the auto-review options are in Settings → AI.
            </p>
          </>
        )}

        {step === 4 && (
          <>
            <h2>Desktop</h2>
            <label className="toggle">
              <input
                type="checkbox"
                checked={autostart}
                onChange={(e) =>
                  void api.setAutostart(e.target.checked).then(setAutostart, (x) => setError(String(x)))
                }
              />
              <span className="switch" />
              <span>
                Start with Windows
                <span className="muted small-text block">
                  Starts quietly in the tray, so every game is tracked and analyzed without you opening
                  anything.
                </span>
              </span>
            </label>
            <label className="toggle">
              <input
                type="checkbox"
                checked={settings.desktop.close_to_tray}
                onChange={(e) =>
                  void onSave({
                    ...settings,
                    desktop: { ...settings.desktop, close_to_tray: e.target.checked },
                  })
                }
              />
              <span className="switch" />
              <span>Keep running in the tray when I close the window</span>
            </label>
            <div className="field">
              <label>Compare me with</label>
              <select
                value={settings.benchmarks.tier}
                onChange={(e) =>
                  void onSave({
                    ...settings,
                    benchmarks: {
                      ...settings.benchmarks,
                      tier: e.target.value as AppSettings["benchmarks"]["tier"],
                    },
                  })
                }
              >
                {TIERS.map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.label}
                  </option>
                ))}
              </select>
              <span className="muted small-text">
                Rift Coach samples games of players at this rank in the background (never during your games)
                to show what the next level looks like on your champions.
              </span>
            </div>
          </>
        )}

        {step === 5 && (
          <>
            <h2>You're set</h2>
            <ul>
              <li>Your games are downloading; the Dashboard fills in as they arrive.</li>
              <li>
                Start a game: the <b>Live</b> page shows your CS pacing and build progress, and the game is
                analyzed a few minutes after it ends.
              </li>
              <li>
                <b>Champions</b>, <b>Patterns</b> and <b>Benchmarks</b> follow the champions you play most.
              </li>
              <li>The Guide (bottom of the menu) explains every number.</li>
            </ul>
          </>
        )}

        {error && <div className="banner error">{error}</div>}
        <div className="setup-nav">
          {step > 0 && step < STEPS.length - 1 && (
            <button className="secondary" onClick={back}>
              Back
            </button>
          )}
          <div className="spacer" />
          {step === 0 && (
            <button className="link" onClick={finish}>
              Skip setup
            </button>
          )}
          {step < STEPS.length - 1 ? (
            <button onClick={next}>
              {step === 0 ? "Get started" : [1, 2, 3].includes(step) ? "Next" : "Continue"}
            </button>
          ) : (
            <button disabled={busy} onClick={finish}>
              Open Rift Coach
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
