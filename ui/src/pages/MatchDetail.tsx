import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { DeathMap } from "../components/DeathMap";
import { LineChart, type ChartSeries } from "../components/LineChart";
import { ReviewBody } from "../components/ReviewBody";
import { ChampionIcon, ItemIcon, SpellIcon, useChampionName } from "../lib/assets";
import { clock, dateTime, formatMetric, pctTone, roleLabel, signed, ZONE_LABEL } from "../lib/format";
import type {
  Comparison,
  MatchDetail as Detail,
  GoalInput,
  MetricGroup,
  MetricView,
  Report,
  ScoreRow,
} from "../types";

interface Props {
  matchId: string;
  onBack: () => void;
  onLookup: (riotId: string) => void;
  onOpenSettings: () => void;
  onTrackFocus: (draft: GoalInput) => void;
  /** Bumped by the shell when new data for this match may exist. */
  refreshKey: number;
}

export function MatchDetail({ matchId, onBack, onLookup, onOpenSettings, onTrackFocus, refreshKey }: Props) {
  const [detail, setDetail] = useState<Detail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api.matchDetail(matchId).then(
      (d) => alive && (setDetail(d), setError(null)),
      (e) => alive && setError(String(e)),
    );
    return () => {
      alive = false;
    };
  }, [matchId, refreshKey]);

  if (error)
    return (
      <div className="page">
        <button className="secondary" onClick={onBack}>
          ← Back
        </button>
        <div className="banner error">{error}</div>
      </div>
    );
  if (!detail) return <div className="page muted">Loading match…</div>;

  return (
    <div className="page match-detail">
      <Header d={detail} onBack={onBack} />
      <ReviewPanel
        d={detail}
        onOpenSettings={onOpenSettings}
        onReport={(r) => setDetail({ ...detail, report: r })}
        onTrackFocus={onTrackFocus}
      />
      {detail.patterns.length > 0 && <PatternsPanel d={detail} />}
      {!detail.is_standard_rift && (
        <div className="banner warn">
          {detail.summary.is_remake ? "Remake" : detail.queue_name}: shown for reference, excluded from
          baselines and comparisons.
        </div>
      )}
      <MetricsPanel d={detail} />
      {detail.series && <ChartsPanel d={detail} />}
      {detail.events.length > 0 && <MapPanel d={detail} />}
      <Scoreboard d={detail} onLookup={onLookup} />
    </div>
  );
}

function Header({ d, onBack }: { d: Detail; onBack: () => void }) {
  const name = useChampionName();
  const s = d.summary;
  const opp = d.scoreboard.find((p) => p.participant_id === d.opponent_id);
  const result = s.is_remake ? "Remake" : s.win ? "Victory" : "Defeat";
  return (
    <section className="panel match-header">
      <button className="secondary" onClick={onBack}>
        ← Matches
      </button>
      <ChampionIcon id={s.champion_id} patch={s.patch} size={64} />
      <div className="mh-main">
        <div className={`mh-result ${s.is_remake ? "remake" : s.win ? "win" : "loss"}`}>{result}</div>
        <h1>
          {name(s.champion_id, s.champion_name)} <span className="muted">{roleLabel(s.team_position)}</span>
        </h1>
        <div className="muted">
          {d.queue_name} · {clock(s.duration_s)} · patch {s.patch} · {dateTime(s.game_start_ms)}
        </div>
      </div>
      <div className="mh-kda">
        <div className="big">
          {s.kills} / <span className="deaths">{s.deaths}</span> / {s.assists}
        </div>
        <div className="muted">
          {s.cs} CS ({(s.cs / Math.max(1, s.duration_s / 60)).toFixed(1)}/min) · {s.vision_score} vision
        </div>
      </div>
      {opp && (
        <div className="mh-vs">
          <span className="muted">vs</span>
          <ChampionIcon id={opp.champion_id} patch={s.patch} size={40} />
          <span>{name(opp.champion_id, opp.champion_name)}</span>
        </div>
      )}
    </section>
  );
}

// ---------- AI review ----------

function ReviewPanel({
  d,
  onOpenSettings,
  onReport,
  onTrackFocus,
}: {
  d: Detail;
  onOpenSettings: () => void;
  onReport: (r: Report) => void;
  onTrackFocus: (draft: GoalInput) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [facts, setFacts] = useState<string | null>(null);
  const r = d.report;
  const labels = useMemo(() => {
    const m = new Map<string, string>(d.comparison.metrics.map((x) => [x.key, x.label]));
    d.patterns.forEach((p) => m.set(`pattern:${p.kind}`, p.title));
    return m;
  }, [d]);
  const labelOf = (id: string) =>
    labels.get(id) ??
    (id.startsWith("death_")
      ? id.replace("death_", "death #")
      : id.startsWith("goal_")
        ? "your goal"
        : id.replace("pattern:", ""));

  const run = async (force: boolean) => {
    setBusy(true);
    setError(null);
    try {
      onReport(await api.generateReview(d.summary.match_id, force));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const showFacts = async () => {
    if (facts) return setFacts(null);
    try {
      setFacts(JSON.stringify(await api.reviewFacts(d.summary.match_id), null, 2));
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <section className="panel review">
      <div className="panel-head">
        <h2>
          <span className="spark">✦</span> AI coach
        </h2>
        <div className="spacer" />
        {r && (
          <span className="muted small-text">
            {r.model} · {dateTime(r.created_at)}
          </span>
        )}
        <button className="secondary small" onClick={showFacts}>
          {facts ? "Hide facts" : "Facts sent"}
        </button>
        <button disabled={busy || d.summary.is_remake} onClick={() => run(!!r)}>
          {busy ? "Thinking…" : r ? "Regenerate" : "Review this game"}
        </button>
      </div>
      {error && (
        <div className="banner error">
          {error}{" "}
          {/key|model|provider/i.test(error) && (
            <button className="link" onClick={onOpenSettings}>
              Open AI settings
            </button>
          )}
        </div>
      )}
      {facts && <pre className="facts">{facts}</pre>}
      {!r && !busy && !error && (
        <p className="muted">
          Get a grounded review of this game: what went well, what to fix, and one focus for your next games.
          The AI sees only the numbers on this page (see "Facts sent"), and every number it writes is checked
          against them.
        </p>
      )}
      {r && <ReviewBody report={r} labelOf={labelOf} onTrackFocus={onTrackFocus} />}
    </section>
  );
}

// ---------- patterns in this game ----------

function PatternsPanel({ d }: { d: Detail }) {
  return (
    <section className="panel">
      <h2>Patterns in this game</h2>
      <div className="match-patterns">
        {d.patterns.map((p) => (
          <div key={p.kind} className={`signal ${p.polarity === "weakness" ? "weak" : "strong"}`}>
            <div className="signal-head">
              <b>{p.title}</b>
              <span className="spacer" />
              {p.recurring ? (
                <span className={`tag ${p.polarity === "weakness" ? "warn" : "ok"}`}>
                  habit: {p.recurring.games_affected} of your last {p.recurring.games_considered} games
                </span>
              ) : (
                <span className="tag">one-off</span>
              )}
            </div>
            <p className="muted small-text">
              {p.what}.
              {p.recurring?.win_rate_affected != null &&
                p.recurring.win_rate_other != null &&
                ` You win ${p.recurring.win_rate_affected}% of games where this happens, ${p.recurring.win_rate_other}% otherwise.`}
            </p>
            {p.evidence.length > 0 && (
              <ul className="evidence-list">
                {p.evidence.map((e, i) => (
                  <li key={i}>
                    {e.t_s !== null && <span className="mono">{clock(e.t_s)} </span>}
                    {e.note}
                  </li>
                ))}
              </ul>
            )}
          </div>
        ))}
      </div>
    </section>
  );
}

// ---------- metrics ----------

const GROUPS: { id: MetricGroup; label: string }[] = [
  { id: "laning", label: "Laning" },
  { id: "macro", label: "Macro" },
  { id: "fighting", label: "Fighting" },
  { id: "overall", label: "Economy" },
  { id: "vision", label: "Vision" },
  { id: "items", label: "Items" },
];

function MetricsPanel({ d }: { d: Detail }) {
  const [group, setGroup] = useState<MetricGroup>("laning");
  const pops = d.comparison.populations;
  const rows = d.comparison.metrics.filter((m) => m.group === group);
  return (
    <section className="panel">
      <div className="panel-head">
        <h2>Metrics</h2>
        <div className="tabs">
          {GROUPS.map((g) => (
            <button key={g.id} className={`tab ${group === g.id ? "on" : ""}`} onClick={() => setGroup(g.id)}>
              {g.label}
            </button>
          ))}
        </div>
      </div>
      <table className="metrics">
        <thead>
          <tr>
            <th>Metric</th>
            <th className="num">This game</th>
            <th title="Other players in this role across your stored games">vs {pops.role_label}</th>
            <th title={`Your previous games in this role (look-back window: ${pops.baseline_games})`}>
              vs {pops.personal_label}
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((m) => (
            <MetricRow key={m.key} m={m} />
          ))}
          {rows.length === 0 && (
            <tr>
              <td colSpan={4} className="muted">
                Not available for this game{d.series ? "" : " (no timeline stored)"}.
              </td>
            </tr>
          )}
        </tbody>
      </table>
      <p className="muted small-text">
        "Better than X%" compares with other players in your own games, mostly your lane opponents (roughly
        your skill bracket). Lane diffs are only compared with your own games, since your opponents' diffs
        mirror yours. Higher-rank benchmarks arrive in Phase 5.
      </p>
    </section>
  );
}

function MetricRow({ m }: { m: MetricView }) {
  const fmt = (v: number) => formatMetric(v, m.unit, m.decimals);
  return (
    <tr>
      <td title={m.description}>
        {m.label}
        {m.approximate && (
          <span className="approx" title="Approximate: from once-per-minute snapshots">
            {" "}
            ≈
          </span>
        )}
      </td>
      <td className="num strong">
        {m.key.includes("diff") && m.value > 0 ? "+" : ""}
        {fmt(m.value)}
      </td>
      <td>
        {m.zero_sum ? (
          <span
            className="muted"
            title="Your lane opponents' diffs mirror yours, so this compares only with your own games"
          >
            compared with your games only
          </span>
        ) : (
          <Pct c={m.vs_role ?? m.vs_champion} fmt={fmt} direction={m.higher_is_better} />
        )}
      </td>
      <td>
        <Pct c={m.vs_personal} fmt={fmt} direction={m.higher_is_better} />
      </td>
    </tr>
  );
}

function Pct({
  c,
  fmt,
  direction,
}: {
  c: Comparison | null;
  fmt: (v: number) => string;
  direction: boolean | null;
}) {
  if (direction === null) return <span className="muted">-</span>;
  if (!c) return <span className="muted">not enough data</span>;
  const p = Math.round(c.better_than_pct);
  return (
    <div className="pct" title={`Median ${fmt(c.median)} · ${c.n} games`}>
      <div className={`pct-bar ${pctTone(p)}`}>
        <span style={{ width: `${p}%` }} />
      </div>
      <span className="pct-text">
        better than <b>{p}%</b> <span className="muted">· med {fmt(c.median)}</span>
      </span>
    </div>
  );
}

// ---------- charts ----------

type ChartKind = "gold" | "xp" | "cs" | "team";

function ChartsPanel({ d }: { d: Detail }) {
  const [kind, setKind] = useState<ChartKind>("gold");
  const [mode, setMode] = useState<"diff" | "abs">("diff");
  const s = d.series;
  if (!s) return null;
  const me = s.by_participant[String(d.participant_id)];
  const opp = d.opponent_id ? s.by_participant[String(d.opponent_id)] : undefined;
  const myTeam = d.scoreboard.find((p) => p.is_subject)?.team_id ?? 100;
  const deathMarkers = d.deaths.map((x) => ({
    x: x.t_s / 60,
    label: `Death at ${clock(x.t_s)}`,
    color: "#d9605a",
  }));

  let series: ChartSeries[] = [];
  let zero = false;
  if (kind === "team") {
    const teamGold = (team: number) =>
      s.minutes.map((_, i) =>
        d.scoreboard
          .filter((p) => p.team_id === team)
          .reduce((sum, p) => sum + (s.by_participant[String(p.participant_id)]?.gold[i] ?? 0), 0),
      );
    const mine = teamGold(myTeam);
    const theirs = teamGold(myTeam === 100 ? 200 : 100);
    series = [
      { label: "Team gold lead", color: "#d6ac52", values: mine.map((v, i) => v - (theirs[i] ?? 0)) },
    ];
    zero = true;
  } else if (me) {
    const key = kind;
    if (mode === "diff" && opp) {
      series = [
        {
          label: `${key.toUpperCase()} vs lane opponent`,
          color: "#40c8c4",
          values: me[key].map((v, i) => v - (opp[key][i] ?? 0)),
        },
      ];
      zero = true;
    } else {
      series = [{ label: "You", color: "#40c8c4", values: me[key] }];
      if (opp) series.push({ label: "Lane opponent", color: "#d9605a", values: opp[key], dashed: true });
    }
  }

  return (
    <section className="panel">
      <div className="panel-head">
        <h2>Timeline</h2>
        <div className="tabs">
          {(["gold", "xp", "cs", "team"] as ChartKind[]).map((k) => (
            <button key={k} className={`tab ${kind === k ? "on" : ""}`} onClick={() => setKind(k)}>
              {k === "team" ? "Team gold" : k.toUpperCase()}
            </button>
          ))}
        </div>
        {kind !== "team" && opp && (
          <div className="tabs">
            <button className={`tab ${mode === "diff" ? "on" : ""}`} onClick={() => setMode("diff")}>
              Difference
            </button>
            <button className={`tab ${mode === "abs" ? "on" : ""}`} onClick={() => setMode("abs")}>
              Both
            </button>
          </div>
        )}
      </div>
      <LineChart
        x={s.minutes}
        series={series}
        zeroBand={zero}
        markers={deathMarkers}
        formatY={(v) => (zero ? signed(Math.round(v)) : Math.round(v).toLocaleString())}
      />
      <p className="muted small-text">Red lines mark your deaths. Values are sampled once per minute.</p>
    </section>
  );
}

// ---------- map ----------

function MapPanel({ d }: { d: Detail }) {
  const myTeam = d.scoreboard.find((p) => p.is_subject)?.team_id ?? 100;
  const champ = (pid: number | null) =>
    d.scoreboard.find((p) => p.participant_id === pid)?.champion_name ?? "?";
  return (
    <section className="panel map-panel">
      <h2>Map</h2>
      <div className="map-grid">
        <DeathMap
          patch={d.summary.patch}
          events={d.events}
          me={d.participant_id}
          myTeam={myTeam}
          durationS={d.summary.duration_s}
          players={d.scoreboard}
        />
        <div className="death-list">
          <h3>Your deaths ({d.deaths.length})</h3>
          {d.deaths.length === 0 && <p className="muted">Deathless. Nice.</p>}
          {d.deaths.map((x, i) => (
            <div key={i} className="death">
              <span className="mono">{clock(x.t_s)}</span>
              <span>
                {x.killer_id && x.killer_id > 0 ? champ(x.killer_id) : "Tower/minions"}
                {x.enemies_involved > 1 && <span className="tag">{x.enemies_involved} enemies</span>}
                {x.before_14 && <span className="tag warn">laning phase</span>}
                <span className="tag">{ZONE_LABEL[x.zone]} ≈</span>
                {x.enemy_objective && <span className="tag warn">enemy took {x.enemy_objective}</span>}
              </span>
              <span className="muted small-text">
                {x.gold_vs_opponent !== null && `${signed(x.gold_vs_opponent)}g vs opp · `}
                {x.unspent_gold !== null && `${x.unspent_gold}g unspent ≈`}
                {x.shutdown_bounty > 0 && ` · gave ${x.shutdown_bounty}g shutdown`}
              </span>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

// ---------- scoreboard ----------

function Scoreboard({ d, onLookup }: { d: Detail; onLookup: (riotId: string) => void }) {
  const name = useChampionName();
  const teams = [100, 200].map((t) => d.scoreboard.filter((p) => p.team_id === t));
  const maxDmg = Math.max(1, ...d.scoreboard.map((p) => p.damage));
  return (
    <section className="panel">
      <h2>Scoreboard</h2>
      {teams.map((team, i) => (
        <table key={i} className={`scoreboard ${team[0]?.win ? "won" : "lost"}`}>
          <thead>
            <tr>
              <th>
                {team[0]?.win ? "Victory" : "Defeat"} · {i === 0 ? "Blue" : "Red"} side
              </th>
              <th className="num">KDA</th>
              <th className="num">CS</th>
              <th className="num">Gold</th>
              <th>Damage</th>
              <th className="num">Vision</th>
              <th>Items</th>
            </tr>
          </thead>
          <tbody>
            {team.map((p) => (
              <PlayerRow
                key={p.participant_id}
                p={p}
                patch={d.summary.patch}
                maxDmg={maxDmg}
                name={name}
                onLookup={onLookup}
              />
            ))}
          </tbody>
        </table>
      ))}
    </section>
  );
}

function PlayerRow({
  p,
  patch,
  maxDmg,
  name,
  onLookup,
}: {
  p: ScoreRow;
  patch: string;
  maxDmg: number;
  name: (id: number, fallback: string) => string;
  onLookup: (riotId: string) => void;
}) {
  return (
    <tr className={p.is_subject ? "me" : ""}>
      <td>
        <span className="champ">
          <ChampionIcon id={p.champion_id} patch={patch} size={30} />
          <span className="spells">
            {p.summoner_spells.map((s, i) => (
              <SpellIcon key={i} id={s} patch={patch} size={14} />
            ))}
          </span>
          <span className="who">
            <b>{name(p.champion_id, p.champion_name)}</b>
            {p.riot_id && !p.is_subject ? (
              <button
                className="link small-text"
                title="Look up this player"
                onClick={() => onLookup(p.riot_id!)}
              >
                {p.riot_id}
              </button>
            ) : (
              <span className="muted small-text">{p.riot_id ?? ""}</span>
            )}
          </span>
        </span>
      </td>
      <td className="num">
        {p.kills}/<span className="deaths">{p.deaths}</span>/{p.assists}
      </td>
      <td className="num">{p.cs}</td>
      <td className="num">{(p.gold / 1000).toFixed(1)}k</td>
      <td>
        <div className="dmg" title={p.damage.toLocaleString()}>
          <span style={{ width: `${(p.damage / maxDmg) * 100}%` }} />
          <em>{(p.damage / 1000).toFixed(1)}k</em>
        </div>
      </td>
      <td className="num">{p.vision_score}</td>
      <td>
        <span className="items">
          {p.items.map((it, i) => (
            <ItemIcon key={i} id={it} patch={patch} size={22} />
          ))}
        </span>
      </td>
    </tr>
  );
}
