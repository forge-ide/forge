import type { Component } from 'solid-js';
import { ProviderPanel } from './Dashboard/ProviderPanel';
import { SessionsPanel } from './Dashboard/SessionsPanel';
import './Dashboard.css';

export const Dashboard: Component = () => {
  return (
    <main class="dashboard">
      <h1 class="dashboard__title">Forge — Dashboard</h1>
      <ProviderPanel />
      <SessionsPanel />
    </main>
  );
};
