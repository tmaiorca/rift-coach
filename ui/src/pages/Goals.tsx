import { useEffect, useState } from "react";
import { api } from "../api";
import { GoalCard, GoalForm } from "../components/Goals";
import type { Goal, GoalInput, MetricDef } from "../types";

export function Goals({
  refreshKey,
  draft,
  onDraftUsed,
  onOpenMatch,
}: {
  refreshKey: number;
  /** Prefill from a pattern or the AI coach. */
  draft: GoalInput | null;
  onDraftUsed: () => void;
  onOpenMatch: (id: string) => void;
}) {
  const [goals, setGoals] = useState<Goal[] | null>(null);
  const [defs, setDefs] = useState<MetricDef[]>([]);
  const [showForm, setShowForm] = useState(false);
  const formOpen = showForm || draft !== null;
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    api.listGoals(true).then(setGoals, (e) => setError(String(e)));
  }, [refreshKey, tick]);
  useEffect(() => {
    api.metricDefinitions().then(setDefs, () => {});
  }, []);

  const reload = () => setTick((t) => t + 1);
  const act = async (f: () => Promise<void>) => {
    try {
      await f();
      reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const active = goals?.filter((g) => g.status === "active") ?? [];
  const ended = goals?.filter((g) => g.status !== "active") ?? [];

  return (
    <div className="page goals">
      <section className="panel">
        <div className="panel-head">
          <h2>Focus goals</h2>
          <div className="spacer" />
          {!formOpen && <button onClick={() => setShowForm(true)}>New goal</button>}
        </div>
        <p className="muted">
          Pick one or two habits and track them game by game. Progress counts your Summoner's Rift games
          played after the goal starts; the AI coach comments on it in reviews.
        </p>
        {error && <div className="banner error">{error}</div>}
        {formOpen && (
          <GoalForm
            key={draft ? JSON.stringify(draft) : "new"}
            defs={defs}
            draft={draft}
            onCreate={async (input) => {
              await api.createGoal(input);
              setShowForm(false);
              onDraftUsed();
              reload();
            }}
            onCancel={() => {
              setShowForm(false);
              onDraftUsed();
            }}
          />
        )}
      </section>

      <section className="panel">
        <h2>Active</h2>
        {goals && active.length === 0 && (
          <p className="muted">
            No active goal. Start one here, or use "Make this a goal" on the Patterns page or "Track as goal"
            in an AI review.
          </p>
        )}
        {active.map((g) => (
          <GoalCard
            key={g.id}
            goal={g}
            onOpenMatch={onOpenMatch}
            onStatus={(s) => act(() => api.setGoalStatus(g.id, s))}
            onDelete={() => act(() => api.deleteGoal(g.id))}
          />
        ))}
      </section>

      {ended.length > 0 && (
        <section className="panel">
          <h2>History</h2>
          {ended.map((g) => (
            <GoalCard
              key={g.id}
              goal={g}
              onOpenMatch={onOpenMatch}
              onStatus={(s) => act(() => api.setGoalStatus(g.id, s))}
              onDelete={() => act(() => api.deleteGoal(g.id))}
            />
          ))}
        </section>
      )}
    </div>
  );
}
