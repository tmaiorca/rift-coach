import type { AccountView, KeyStatus, StatusView } from "../types";

const KEY_LABEL: Record<KeyStatus, string> = {
  unknown: "Riot key loaded",
  ok: "Riot key OK",
  missing: "No Riot key",
  rejected: "Riot key rejected",
};

const KEY_HELP: Partial<Record<KeyStatus, string>> = {
  missing: "Add your Riot API key in Settings (or set the RIOT_API_KEY environment variable).",
  rejected: "Riot rejected the key (invalid, revoked, or expired). Update it in Settings.",
};

interface Props {
  status: StatusView | null;
  accounts: AccountView[];
  onSwitch: (accountId: number) => void;
  onReloadKey: () => void;
  onOpenSettings: () => void;
}

export function StatusBar({ status, accounts, onSwitch, onReloadKey, onOpenSettings }: Props) {
  const key = status?.engine.key_status ?? "unknown";
  const account = status?.account;
  return (
    <header className="statusbar">
      {accounts.length > 0 ? (
        <select
          className="account-switch"
          value={account?.id ?? ""}
          onChange={(e) => onSwitch(Number(e.target.value))}
          aria-label="Player"
        >
          {accounts.map((a) => (
            <option key={a.id} value={a.id}>
              {a.game_name}#{a.tag_line} · {a.platform.toUpperCase()}
              {a.is_primary ? " (you)" : ""}
            </option>
          ))}
        </select>
      ) : (
        <span className="muted">No player tracked yet</span>
      )}
      <div className="spacer" />
      <span className={`pill key-${key}`} title={KEY_HELP[key] ?? ""}>
        {KEY_LABEL[key]}
      </span>
      {KEY_HELP[key] && (
        <>
          <button className="secondary small" onClick={onReloadKey}>
            Reload key
          </button>
          <button className="small" onClick={onOpenSettings}>
            Settings
          </button>
          <div className="banner warn">{KEY_HELP[key]}</div>
        </>
      )}
    </header>
  );
}
