import type { Component } from 'solid-js';
import { Router, Route } from '@solidjs/router';
import { Dashboard } from './routes/Dashboard';
import { SessionWindow } from './routes/Session/SessionWindow';
import { AgentMonitor } from './routes/AgentMonitor';

export const App: Component = () => {
  return (
    <Router>
      <Route path="/" component={Dashboard} />
      <Route path="/session/:id" component={SessionWindow} />
      {/* F-140: session-scoped when we have an id (sub-agents + bg agents
          live per-session), session-agnostic fallback for the dashboard
          entry point until F-138 wires the status-bar agent badge. */}
      <Route path="/agents" component={AgentMonitor} />
      <Route path="/agents/:id" component={AgentMonitor} />
    </Router>
  );
};
