import { ForgeAgentTask, ForgeAgentStep } from '../../../../services/forge/common/forgeAgentTypes.js';

const bannerStatusDots = new WeakMap<HTMLElement, HTMLElement>();
const bannerTurnCounts = new WeakMap<HTMLElement, HTMLElement>();
const bannerStepsContainers = new WeakMap<HTMLElement, HTMLElement>();

export function renderAgentBanner(task: ForgeAgentTask): HTMLElement {
	const banner = document.createElement('div');
	banner.className = 'forge-agent-banner';
	banner.dataset.agentId = task.id;

	const header = document.createElement('div');
	header.className = 'forge-agent-banner-header';

	const statusDot = document.createElement('span');
	statusDot.className = `forge-agent-status ${task.status}`;
	header.appendChild(statusDot);
	bannerStatusDots.set(banner, statusDot);

	const name = document.createElement('span');
	name.className = 'forge-agent-name';
	name.textContent = task.name;
	header.appendChild(name);

	const turnCount = document.createElement('span');
	turnCount.className = 'forge-agent-turns';
	turnCount.textContent = `${task.currentTurn}/${task.maxTurns}`;
	header.appendChild(turnCount);
	bannerTurnCounts.set(banner, turnCount);

	banner.appendChild(header);

	const stepsContainer = document.createElement('div');
	stepsContainer.className = 'forge-agent-steps collapsed';
	bannerStepsContainers.set(banner, stepsContainer);

	const toggle = document.createElement('button');
	toggle.className = 'forge-agent-steps-toggle';
	toggle.textContent = 'Steps';
	toggle.addEventListener('click', () => {
		stepsContainer.classList.toggle('collapsed');
	});
	banner.appendChild(toggle);
	banner.appendChild(stepsContainer);

	return banner;
}

export function updateAgentBanner(banner: HTMLElement, task: ForgeAgentTask): void {
	const statusDot = bannerStatusDots.get(banner);
	if (statusDot) {
		statusDot.className = `forge-agent-status ${task.status}`;
	}

	const turnCount = bannerTurnCounts.get(banner);
	if (turnCount) {
		turnCount.textContent = `${task.currentTurn}/${task.maxTurns}`;
	}
}

export function appendAgentStep(banner: HTMLElement, step: ForgeAgentStep): void {
	const container = bannerStepsContainers.get(banner);
	if (!container) {
		return;
	}

	const stepEl = document.createElement('div');
	stepEl.className = `forge-agent-step ${step.status}`;
	stepEl.dataset.callId = step.toolCallId;

	const dot = document.createElement('span');
	dot.className = `forge-tool-call-status ${step.status}`;
	stepEl.appendChild(dot);

	const label = document.createElement('span');
	label.textContent = `${step.toolName}(${JSON.stringify(step.input).slice(0, 60)})`;
	stepEl.appendChild(label);

	container.appendChild(stepEl);
}
