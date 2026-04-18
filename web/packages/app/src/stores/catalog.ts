import { createStore } from 'solid-js/store';
import type { AgentId, ProviderId } from '@forge/ipc';

export interface ProviderSummary {
  id: ProviderId;
  name: string;
}

export interface McpServerInfo {
  name: string;
}

export interface SkillInfo {
  name: string;
}

export interface AgentInfo {
  id: AgentId;
  name: string;
}

export interface ContainerSummary {
  name: string;
}

export const [providers, setProviders] = createStore<ProviderSummary[]>([]);
export const [mcpServers, setMcpServers] = createStore<McpServerInfo[]>([]);
export const [skills, setSkills] = createStore<SkillInfo[]>([]);
export const [agents, setAgents] = createStore<AgentInfo[]>([]);
export const [containers, setContainers] = createStore<ContainerSummary[]>([]);
