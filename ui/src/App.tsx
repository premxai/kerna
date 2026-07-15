import { FormEvent, useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

interface Task {
  id: string;
  goal: string;
  status: string;
  created_at: string;
}

interface Memory {
  id: string;
  content: string;
}

interface TaskEvent {
  timestamp: string;
  event_type: string;
  severity: string;
  tool: string | null;
  policy_decision: string | null;
  redaction_status: string | null;
}

interface ConnectorStatus {
  name: string;
  enabled: boolean;
  secrets_needed: number;
  secrets_ready: boolean;
  last_activity: string | null;
  last_result: "success" | "failed" | null;
}

interface RoutineStatus {
  index: number;
  name: string;
  cron: string;
  goal: string;
  enabled: boolean;
  allowed_tools: string[];
  policy_ready: boolean;
}

interface PendingApproval {
  id: string;
  task_id: string;
  tool: string;
  args_json: string;
  created_at: string;
}

function formatTime(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : new Intl.DateTimeFormat(undefined, {
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      }).format(date);
}

function humanizeEvent(event: string) {
  return event.replace(/[._]/g, " ");
}

function displayError(cause: unknown) {
  const message = String(cause);
  if (message.includes("invoke")) {
    return "This browser preview is not connected to the Kerna desktop service. Launch the desktop app to view local tasks and controls.";
  }
  return message;
}

function connectorDetail(connector: ConnectorStatus) {
  const setup = connector.secrets_needed
    ? `${connector.secrets_needed} secret${connector.secrets_needed === 1 ? "" : "s"} declared · ${connector.secrets_ready ? "setup complete" : "setup needed"}`
    : "no secret required";
  if (!connector.last_activity || !connector.last_result) return `${setup} · not checked yet`;
  return `${setup} · last tool call ${connector.last_result === "success" ? "succeeded" : "failed"} ${formatTime(connector.last_activity)}`;
}

function App() {
  const [goal, setGoal] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [memories, setMemories] = useState<Memory[]>([]);
  const [connectors, setConnectors] = useState<ConnectorStatus[]>([]);
  const [routines, setRoutines] = useState<RoutineStatus[]>([]);
  const [approvals, setApprovals] = useState<PendingApproval[]>([]);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [events, setEvents] = useState<TaskEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [runningRoutine, setRunningRoutine] = useState<number | null>(null);

  const loadData = useCallback(async () => {
    try {
      const [dbTasks, dbMemories, configuredConnectors, configuredRoutines, pendingApprovals] = await Promise.all([
        invoke<Task[]>("get_tasks"),
        invoke<Memory[]>("get_memories"),
        invoke<ConnectorStatus[]>("get_connectors"),
        invoke<RoutineStatus[]>("get_routines"),
        invoke<PendingApproval[]>("get_pending_approvals"),
      ]);
      setTasks(dbTasks);
      setMemories(dbMemories);
      setConnectors(configuredConnectors);
      setRoutines(configuredRoutines);
      setApprovals(pendingApprovals);
      setSelectedTaskId((current) => current ?? dbTasks[0]?.id ?? null);
      setError(null);
    } catch (cause) {
      setError(displayError(cause));
    }
  }, []);

  useEffect(() => {
    void loadData();
    const interval = window.setInterval(() => void loadData(), 5000);
    return () => window.clearInterval(interval);
  }, [loadData]);

  useEffect(() => {
    if (!selectedTaskId) {
      setEvents([]);
      return;
    }
    void invoke<TaskEvent[]>("get_task_events", { taskId: selectedTaskId })
      .then(setEvents)
      .catch((cause) => setError(displayError(cause)));
  }, [selectedTaskId]);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const trimmedGoal = goal.trim();
    if (!trimmedGoal) return;

    setIsSubmitting(true);
    setNotice(null);
    try {
      const result = await invoke<string>("run_goal", { goal: trimmedGoal });
      setGoal("");
      setNotice(result);
      window.setTimeout(() => void loadData(), 500);
    } catch (cause) {
      setError(displayError(cause));
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleRoutineRun = async (routine: RoutineStatus) => {
    setRunningRoutine(routine.index);
    setNotice(null);
    try {
      const result = await invoke<string>("run_routine", { index: routine.index });
      setNotice(result);
      window.setTimeout(() => void loadData(), 500);
    } catch (cause) {
      setError(displayError(cause));
    } finally {
      setRunningRoutine(null);
    }
  };

  const decideApproval = async (approval: PendingApproval, approved: boolean) => {
    try {
      await invoke("decide_approval", { id: approval.id, approved });
      setNotice(approved ? `Approved ${approval.tool}.` : `Denied ${approval.tool}.`);
      await loadData();
    } catch (cause) {
      setError(displayError(cause));
    }
  };

  const selectedTask = tasks.find((task) => task.id === selectedTaskId) ?? null;

  return (
    <div className="shell">
      <header className="topbar">
        <div className="brand" aria-label="Kerna trust layer">
          <span className="brand-mark" aria-hidden="true">K</span>
          <span>Kerna</span>
          <small>trust layer</small>
        </div>
        <p className="topbar-copy">Run useful work. Review every consequential action.</p>
      </header>

      <main className="workspace">
        <section className="workbench" aria-labelledby="run-title">
          <div className="section-heading">
            <p className="eyebrow">New task</p>
            <h1 id="run-title">What needs doing?</h1>
            <p>Tools remain governed by your Kerna policy, budget, and approval rules.</p>
          </div>

          <form className="goal-form" onSubmit={handleSubmit}>
            <label htmlFor="goal">Describe the outcome</label>
            <textarea
              id="goal"
              value={goal}
              onChange={(event) => setGoal(event.target.value)}
              placeholder="Prepare a concise brief for today's meetings."
              rows={5}
            />
            <div className="form-footer">
              <span>Approval-required actions pause in your local queue until you decide.</span>
              <button type="submit" disabled={isSubmitting || !goal.trim()}>
                {isSubmitting ? "Starting task…" : "Run task"}
              </button>
            </div>
          </form>

          {notice && <p className="notice" role="status">{notice}</p>}
          {error && <p className="error" role="alert">{error}</p>}

          {approvals.length > 0 && (
            <section className="approval-queue" aria-labelledby="approval-title">
              <p className="eyebrow">Your decision required</p>
              <h2 id="approval-title">Pending actions</h2>
              {approvals.map((approval) => (
                <article key={approval.id}>
                  <strong>{approval.tool}</strong>
                  <time>{formatTime(approval.created_at)}</time>
                  <pre>{approval.args_json}</pre>
                  <div>
                    <button type="button" className="deny" onClick={() => void decideApproval(approval, false)}>Deny</button>
                    <button type="button" className="approve" onClick={() => void decideApproval(approval, true)}>Approve once</button>
                  </div>
                </article>
              ))}
            </section>
          )}

          <div className="section-heading history-heading">
            <p className="eyebrow">Actual task history</p>
            <h2>Recent runs</h2>
          </div>
          <div className="task-list" aria-live="polite">
            {tasks.length === 0 ? (
              <p className="empty-state">Your completed and running tasks will appear here.</p>
            ) : (
              tasks.map((task) => (
                <button
                  className={`task-row ${selectedTaskId === task.id ? "selected" : ""}`}
                  key={task.id}
                  onClick={() => setSelectedTaskId(task.id)}
                  type="button"
                >
                  <span className="task-row-main">
                    <span className="task-goal">{task.goal}</span>
                    <span className="task-time">{formatTime(task.created_at)}</span>
                  </span>
                  <span className={`task-state ${task.status.toLowerCase()}`}>{task.status}</span>
                </button>
              ))
            )}
          </div>
        </section>

        <aside className="inspector" aria-label="Task inspector">
          <section className="inspector-section trace-section">
            <p className="eyebrow">Action receipt</p>
            <h2>{selectedTask ? selectedTask.goal : "Select a task"}</h2>
            {selectedTask && (
              <p className="task-meta">
                <span className={`task-state ${selectedTask.status.toLowerCase()}`}>{selectedTask.status}</span>
                <span>{formatTime(selectedTask.created_at)}</span>
              </p>
            )}

            {selectedTask && events.length === 0 ? (
              <p className="empty-state">No recorded tool events for this task yet.</p>
            ) : (
              <ol className="event-list">
                {events.map((event, index) => (
                  <li key={`${event.timestamp}-${index}`}>
                    <span className={`event-dot ${event.severity}`} aria-hidden="true" />
                    <div>
                      <strong>{humanizeEvent(event.event_type)}</strong>
                      <p>
                        {event.tool && <span>{event.tool} · </span>}
                        {event.policy_decision ?? "recorded"}
                        {event.redaction_status && " · sensitive fields redacted"}
                      </p>
                    </div>
                    <time>{formatTime(event.timestamp)}</time>
                  </li>
                ))}
              </ol>
            )}
          </section>

          <section className="inspector-section memory-section">
            <div className="memory-heading">
              <div>
                <p className="eyebrow">Approved context</p>
                <h2>Memory</h2>
              </div>
              <span>{memories.length}</span>
            </div>
            {memories.length === 0 ? (
              <p className="empty-state">Approved memories are available to future tasks.</p>
            ) : (
              <ul className="memory-list">
                {memories.slice(0, 4).map((memory) => (
                  <li key={memory.id}>{memory.content}</li>
                ))}
              </ul>
            )}
          </section>

          <section className="inspector-section routine-section">
            <p className="eyebrow">Recurring work</p>
            <h2>Routines</h2>
            {routines.length === 0 ? (
              <p className="empty-state">Add a routine from the CLI, then review and activate its narrow policy.</p>
            ) : (
              <ul className="status-list">
                {routines.map((routine) => (
                  <li key={`${routine.index}-${routine.name}-${routine.cron}`}>
                    <div>
                      <strong>{routine.name}</strong>
                      <p title={routine.goal}>{routine.allowed_tools.length} reviewed tool{routine.allowed_tools.length === 1 ? "" : "s"} · {routine.cron}</p>
                    </div>
                    <div className="routine-actions">
                      <span className={`routine-state ${routine.enabled && routine.policy_ready ? "ready" : "paused"}`}>
                        {routine.enabled && routine.policy_ready ? "active" : routine.policy_ready ? "paused" : "review"}
                      </span>
                      <button
                        className="routine-run"
                        type="button"
                        disabled={!routine.policy_ready || runningRoutine === routine.index}
                        onClick={() => void handleRoutineRun(routine)}
                      >
                        {runningRoutine === routine.index ? "Starting…" : "Run once"}
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className="inspector-section connector-section">
            <p className="eyebrow">Configured tools</p>
            <h2>Connectors</h2>
            {connectors.length === 0 ? (
              <p className="empty-state">Install a curated pack or add an MCP connector to begin.</p>
            ) : (
              <ul className="status-list">
                {connectors.map((connector) => (
                  <li key={connector.name}>
                    <div>
                      <strong>{connector.name}</strong>
                      <p>{connectorDetail(connector)}</p>
                    </div>
                    <span className={`routine-state ${connector.enabled && connector.secrets_ready ? connector.last_result === "failed" ? "attention" : "ready" : "paused"}`}>
                      {connector.enabled ? connector.secrets_ready ? connector.last_result === "failed" ? "attention" : "ready" : "needs setup" : "off"}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </aside>
      </main>
    </div>
  );
}

export default App;
