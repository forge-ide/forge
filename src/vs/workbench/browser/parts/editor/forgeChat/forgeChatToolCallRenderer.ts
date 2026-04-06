import { ForgeChatToolCallPart, ForgeChatToolResultPart } from './forgeChatMessageTypes.js';

const statusDotMap = new WeakMap<HTMLElement, HTMLElement>();

export function renderToolCallCard(part: ForgeChatToolCallPart): HTMLElement {
	const card = document.createElement('div');
	card.className = 'forge-tool-call-card';
	card.dataset.callId = part.callId;

	// Header row: status dot + tool name + server name
	const header = document.createElement('div');
	header.className = 'forge-tool-call-header';

	const statusDot = document.createElement('span');
	statusDot.className = `forge-tool-call-status ${part.status}`;
	header.appendChild(statusDot);
	statusDotMap.set(card, statusDot);

	const name = document.createElement('span');
	name.className = 'forge-tool-call-name';
	name.textContent = part.toolName;
	header.appendChild(name);

	const server = document.createElement('span');
	server.className = 'forge-tool-call-server';
	server.textContent = part.serverName;
	header.appendChild(server);

	card.appendChild(header);

	// Collapsible arguments
	const args = document.createElement('div');
	args.className = 'forge-tool-call-args collapsed';

	const argsToggle = document.createElement('button');
	argsToggle.className = 'forge-tool-call-args-toggle';
	argsToggle.textContent = 'Arguments';
	argsToggle.addEventListener('click', () => {
		args.classList.toggle('collapsed');
	});
	card.appendChild(argsToggle);

	const argsContent = document.createElement('pre');
	argsContent.className = 'forge-tool-call-args-content';
	argsContent.textContent = JSON.stringify(part.input, null, 2);
	args.appendChild(argsContent);
	card.appendChild(args);

	return card;
}

export function updateToolCallCard(card: HTMLElement, status: string): void {
	const statusDot = statusDotMap.get(card);
	if (statusDot) {
		statusDot.className = `forge-tool-call-status ${status}`;
	}
}

export function renderToolResultInCard(card: HTMLElement, result: ForgeChatToolResultPart): void {
	const resultEl = document.createElement('div');
	resultEl.className = `forge-tool-call-result${result.isError ? ' error' : ''}`;

	if (result.durationMs !== undefined) {
		const duration = document.createElement('span');
		duration.className = 'forge-tool-call-duration';
		duration.textContent = `${result.durationMs}ms`;
		resultEl.appendChild(duration);
	}

	const preview = document.createElement('pre');
	preview.className = 'forge-tool-call-result-preview';
	const truncated = result.content.length > 200
		? result.content.slice(0, 200) + '…'
		: result.content;
	preview.textContent = truncated;
	resultEl.appendChild(preview);

	card.appendChild(resultEl);
}
