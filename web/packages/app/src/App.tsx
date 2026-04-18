import type { Component } from 'solid-js';
import { Router, Route } from '@solidjs/router';
import { Dashboard } from './routes/Dashboard';

export const App: Component = () => {
  return (
    <Router>
      <Route path="/" component={Dashboard} />
    </Router>
  );
};
