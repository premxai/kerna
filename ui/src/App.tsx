import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { 
  Terminal, 
  BrainCircuit, 
  Settings, 
  Activity, 
  Clock, 
  Play, 
  Database,
  ArrowRight
} from "lucide-react";
import "./App.css";

interface Task {
  id: String;
  goal: String;
  status: String;
}

interface Memory {
  id: String;
  content: String;
}

function App() {
  const [goal, setGoal] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [memories, setMemories] = useState<Memory[]>([]);

  const loadData = async () => {
    try {
      const dbTasks = await invoke<Task[]>("get_tasks");
      setTasks(dbTasks);
      const dbMemories = await invoke<Memory[]>("get_memories");
      setMemories(dbMemories);
    } catch (e) {
      console.error(e);
    }
  };

  useEffect(() => {
    loadData();
    const interval = setInterval(loadData, 2000); // Poll every 2 seconds
    return () => clearInterval(interval);
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!goal.trim()) return;
    
    setIsSubmitting(true);
    try {
      await invoke("run_goal", { goal });
      setGoal("");
      setIsSubmitting(false);
      loadData();
    } catch (err) {
      console.error(err);
      setIsSubmitting(false);
    }
  };

  return (
    <div className="app-layout">
      {/* Sidebar Navigation */}
      <aside className="sidebar">
        <div className="sidebar-logo">
          <div className="logo-icon">
            <BrainCircuit size={20} color="white" />
          </div>
          <h1>AgentOS</h1>
        </div>

        <nav>
          <a href="#" className="nav-item active">
            <Activity size={18} />
            Dashboard
          </a>
          <a href="#" className="nav-item">
            <Database size={18} />
            Memory Explorer
          </a>
          <a href="#" className="nav-item">
            <Clock size={18} />
            Scheduled Jobs
          </a>
          <a href="#" className="nav-item">
            <Terminal size={18} />
            System Console
          </a>
          <a href="#" className="nav-item">
            <Settings size={18} />
            Settings
          </a>
        </nav>

        <div className="stats-grid">
          <div className="stat-box">
            <span className="stat-value">3</span>
            <span className="stat-label">Tasks Today</span>
          </div>
          <div className="stat-box">
            <span className="stat-value">128</span>
            <span className="stat-label">Memories</span>
          </div>
        </div>
      </aside>

      {/* Main Content Area */}
      <main className="main-content">
        <header className="header">
          <h2>Task Runner</h2>
          <div className="status-badge glass-panel">
            <div className="status-dot"></div>
            Daemon Active
          </div>
        </header>

        <div className="dashboard-grid">
          {/* Left Column - Task Input and List */}
          <section className="task-section">
            <form onSubmit={handleSubmit} className="task-input-container">
              <textarea
                className="task-input glass-panel"
                placeholder="What do you want to accomplish?"
                value={goal}
                onChange={(e) => setGoal(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    handleSubmit(e);
                  }
                }}
              />
              <button 
                type="submit" 
                className="task-submit-btn"
                disabled={isSubmitting || !goal.trim()}
              >
                {isSubmitting ? 'Running...' : (
                  <span style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                    Execute <Play size={16} fill="currentColor" />
                  </span>
                )}
              </button>
            </form>

            <div className="card glass-panel" style={{ flex: 1, marginTop: '24px' }}>
              <div className="card-title">
                <Terminal size={20} className="card-title-icon" />
                Recent Tasks
              </div>
              <div className="task-list">
                {tasks.map((task) => (
                  <div key={task.id} className="task-item">
                    <div className="task-header">
                      <span className="task-goal">{task.goal}</span>
                      <span className={`task-status status-${task.status}`}>
                        {task.status}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </section>

          {/* Right Column - Context & Memory */}
          <section className="memory-section">
            <div className="card glass-panel" style={{ height: '100%' }}>
              <div className="card-title">
                <Database size={20} className="card-title-icon" style={{ color: 'var(--accent-purple)' }} />
                Active Context
              </div>
              <p style={{ color: 'var(--text-secondary)', marginBottom: '20px', fontSize: '0.9rem' }}>
                Relevant memories automatically injected into the agent's context window.
              </p>
              
              <div className="memory-list">
                {memories.map((mem, idx) => (
                  <div key={idx} className="memory-item">
                    <div className="memory-content">{mem}</div>
                  </div>
                ))}
              </div>
              
              <button className="glass-panel" style={{ 
                marginTop: 'auto', 
                padding: '12px', 
                display: 'flex', 
                alignItems: 'center', 
                justifyContent: 'center',
                gap: '8px',
                color: 'var(--accent-blue)',
                border: '1px solid rgba(59, 130, 246, 0.3)',
                background: 'rgba(59, 130, 246, 0.05)',
                cursor: 'pointer',
                borderRadius: '8px'
              }}>
                Open Memory Explorer <ArrowRight size={16} />
              </button>
            </div>
          </section>
        </div>
      </main>
    </div>
  );
}

export default App;
