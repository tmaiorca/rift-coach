import type { KeyStatus, StatusView } from "../types";

const KEY_LABEL: Record<KeyStatus, string> = {
  unknown: "Key loaded (not yet used)",
  ok: "Key OK",
  missing: "No API key",
  rejected: "Key rejected",
};

const KEY_HELP: Partial<Record<KeyStatus, string>> = {
  missing: "Set the RIOT_API_KEY user environment variable, then click Reload key.",
  rejected:
    "Riot rejected the key (invalid, revoked, or expired). Update RIOT_API_KEY, then click Reload key.",
};

export function StatusBar({ status, onReloadKey }: { status: StatusView | null; onReloadKey: () => void }) {
  const key = status?.engine.key_status ?? "unknown";
  const account = status?.account;
  return (
    <header className="statusbar">
      <div className="brand">Rift Coach</div>
      <div className="account">
        {account ? (
          <>
            <strong>
              {account.game_name}
              <span className="muted">#{account.tag_line}</span>
            </strong>
            <span className="muted"> · {account.platform.toUpperCase()}</span>
          </>
        ) : (
          <span className="muted">No account tracked yet</span>
        )}
      </div>
      <div className="spacer" />
      <span className={`pill key-${key}`} title={KEY_HELP[key] ?? ""}>
        {KEY_LABEL[key]}
      </span>
      {(key === "missing" || key === "rejected") && (
        <button className="secondary" onClick={onReloadKey}>
          Reload key
        </button>
      )}
      {KEY_HELP[key] && <div className="banner warn">{KEY_HELP[key]}</div>}
    </header>
  );
}
