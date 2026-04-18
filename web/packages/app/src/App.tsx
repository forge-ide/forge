import type { Component } from 'solid-js';
import { Router, Route } from '@solidjs/router';
import { Dashboard } from './routes/Dashboard';
import { SessionWindow } from './routes/Session/SessionWindow';

export const App: Component = () => {
  return (
    <Router>
      <Route path="/" component={Dashboard} />
      <Route path="/session/:id" component={SessionWindow} />
    </Router>
  );
};
