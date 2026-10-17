import { useEffect, useState } from "react";
import { api } from "../api";
import { PLATFORMS } from "../lib/format";
import type { AccountView } from "../types";

interface Props {
  defaultCount: number;
  /** Prefill from "look up this player" on a scoreboard. */
  prefill: string | null;
  refreshKey: number;
  onChanged: () => void;
  onView: () => void;
}

const PRESETS = [20, 50, 100, 200, 500];

export function Players({ defaultCount, prefill, refreshKey, onChanged, onView }: Props) {
  const [accounts, setAccounts] = useState<AccountView[]>([]);
  const [riotId, setRiotId] = useState(prefill ?? "");
  const [platform, setPlatform] = useState("na1");
  const [count, setCount] = useState(defaultCount);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ tone: "error" | "ok"; text: string } | null>(null);
  const [prevPrefill, setPrevPrefill] = useState(prefill);
  if (prefill !== prevPrefill) {
    setPrevPrefill(prefill);
    if (prefill) setRiotId(prefill);
  }

  const load = () => api.listAccounts().then(setAccounts, (e) => setMsg({ tone: "error", text: String(e) }));
  useEffect(() => {
    api.listAccounts().then(setAccounts, (e) => setMsg({ tone: "error", text: String(e) }));
  }, [refreshKey]);

  const act = async (f: () => Promise<unknown>, ok?: string) => {
    setBusy(true);
    setMsg(null);
    try {
      await f();
      if (ok) setMsg({ tone: "ok", text: ok });
      await load();
      onChanged();
    } catch (e) {
      setMsg({ tone: "error", text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const lookup = () =>
    act(async () => {
      const id = await api.startIngest(riotId.trim(), platform, count);
      await api.setActiveAccount(id);
    }, `Fetching up to ${count} matches for ${riotId.trim()}. You're now viewing this player; progress shows on the dashboard.`);

  return (
    <div className="page">
      <section className="panel">
        <h2>Look up a player</h2>
        <p className="muted">
          Track any Riot ID: yours, a smurf, a friend, or someone from your lobby. Their games are fetched in
          the background (Riot allows about 50 games a minute with a personal key).
        </p>
        <div className="form-row">
          <input
            className="grow"
            placeholder="Riot ID, e.g. TapRooted#BTC"
            value={riotId}
            onChange={(e) => setRiotId(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && riotId.includes("#") && lookup()}
            aria-label="Riot ID"
          />
          <select value={platform} onChange={(e) => setPlatform(e.target.value)} aria-label="Region">
            {PLATFORMS.map((p) => (
              <option key={p} value={p}>
                {p.toUpperCase()}
              </option>
            ))}
          </select>
        </div>
        <div className="form-row count-row">
          <label>
            Games to look back: <b>{count}</b>
          </label>
          <input
            type="range"
            min={10}
            max={1000}
            step={10}
            value={count}
            onChange={(e) => setCount(Number(e.target.value))}
          />
          <div className="chips">
            {PRESETS.map((p) => (
              <button key={p} className={`chip ${count === p ? "on" : ""}`} onClick={() => setCount(p)}>
                {p}
              </button>
            ))}
          </div>
          <button disabled={!riotId.includes("#") || busy} onClick={lookup}>
            Look up
          </button>
        </div>
        {msg && <div className={`banner ${msg.tone === "error" ? "error" : "ok"}`}>{msg.text}</div>}
      </section>

      <section className="panel">
        <h2>Tracked players</h2>
        {accounts.length === 0 && <p className="muted">None yet.</p>}
        <div className="accounts">
          {accounts.map((a) => (
            <div key={a.id} className={`account-card ${a.is_active ? "active" : ""}`}>
              <div>
                <b>
                  {a.game_name}
                  <span className="muted">#{a.tag_line}</span>
                </b>{" "}
                <span className="muted">{a.platform.toUpperCase()}</span>
                {a.is_primary && <span className="tag gold">You</span>}
                {a.is_active && <span className="tag">Viewing</span>}
                <div className="muted small-text">
                  {a.counts.matches} matches · {a.counts.with_timeline} timelines
                </div>
              </div>
              <div className="account-actions">
                {!a.is_active && (
                  <button
                    className="secondary small"
                    disabled={busy}
                    onClick={() => act(() => api.setActiveAccount(a.id)).then(onView)}
                  >
                    View
                  </button>
                )}
                <button
                  className="secondary small"
                  disabled={busy}
                  onClick={() =>
                    act(
                      () => api.startIngest(`${a.game_name}#${a.tag_line}`, a.platform, count),
                      `Syncing ${a.game_name}.`,
                    )
                  }
                >
                  Sync {count}
                </button>
                {!a.is_primary && (
                  <button
                    className="secondary small"
                    disabled={busy}
                    onClick={() => act(() => api.setPrimaryAccount(a.id))}
                    title="Mark as your own account"
                  >
                    This is me
                  </button>
                )}
                <button
                  className="secondary small danger"
                  disabled={busy}
                  onClick={() =>
                    window.confirm(
                      `Stop tracking ${a.game_name}#${a.tag_line}? Their AI reports are deleted; stored matches stay.`,
                    ) && act(() => api.removeAccount(a.id))
                  }
                >
                  Remove
                </button>
              </div>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}
