import type { Component } from 'solid-js';
import { A } from '@solidjs/router';
import { ProviderPanel } from './Dashboard/ProviderPanel';
import { SessionsPanel } from './Dashboard/SessionsPanel';
import './Dashboard.css';

export const Dashboard: Component = () => {
  return (
    <main class="dashboard">
      <h1 class="dashboard__title">Forge — Dashboard</h1>
      <nav class="dashboard__nav" aria-label="App navigation">
        <A href="/agents" class="dashboard__nav-link">
          Agent Monitor
        </A>
      </nav>
      <ProviderPanel />
      <SessionsPanel />
    </main>
  );
};
