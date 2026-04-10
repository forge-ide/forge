import { env } from '../../../../base/common/process.js';
import { Disposable } from '../../../../base/common/lifecycle.js';
import { joinPath } from '../../../../base/common/resources.js';
import { URI } from '../../../../base/common/uri.js';
import { IFileService } from '../../../../platform/files/common/files.js';
import { InstantiationType, registerSingleton } from '../../../../platform/instantiation/common/extensions.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { INativeHostService } from '../../../../platform/native/common/native.js';
import { IStorageService, StorageScope, StorageTarget } from '../../../../platform/storage/common/storage.js';
import { IEnvironmentDetectionResult, IForgeOnboardingService } from '../common/forgeOnboardingService.js';

const ONBOARDING_COMPLETE_KEY = 'forge.onboarding.complete';
const MCP_SELECTIONS_KEY = 'forge.onboarding.mcpSelections';

export class ForgeOnboardingServiceImpl extends Disposable implements IForgeOnboardingService {
	declare readonly _serviceBrand: undefined;

	constructor(
		@IStorageService private readonly _storageService: IStorageService,
		@IFileService private readonly _fileService: IFileService,
		@ILogService private readonly _logService: ILogService,
		@INativeHostService private readonly _nativeHostService: INativeHostService,
	) {
		super();
	}

	get onboardingComplete(): boolean {
		return this._storageService.getBoolean(ONBOARDING_COMPLETE_KEY, StorageScope.APPLICATION, false);
	}

	async detectEnvironment(): Promise<IEnvironmentDetectionResult> {
		this._logService.debug('[ForgeOnboardingService] Detecting environment');

		const [vscodeConfig, detectedApiKeys, localModels, npxAvailable] = await Promise.all([
			this._detectVSCodeConfig(),
			this._scanForApiKeys(),
			this._probeLocalModels(),
			this._checkNpx(),
		]);

		return {
			hasVSCodeConfig: vscodeConfig.found,
			vscodeConfigPath: vscodeConfig.path,
			detectedApiKeys,
			ollamaRunning: localModels.ollama,
			lmStudioRunning: localModels.lmStudio,
			npxAvailable,
			vertexEnv: {
				projectId: env['GOOGLE_CLOUD_PROJECT'] || undefined,
				location: env['GOOGLE_CLOUD_LOCATION'] || undefined,
			},
		};
	}

	markComplete(): void {
		this._storageService.store(ONBOARDING_COMPLETE_KEY, true, StorageScope.APPLICATION, StorageTarget.USER);
		this._logService.info('[ForgeOnboardingService] Onboarding marked complete');
	}

	reset(): void {
		this._storageService.remove(ONBOARDING_COMPLETE_KEY, StorageScope.APPLICATION);
		this._logService.info('[ForgeOnboardingService] Onboarding reset');
	}

	async saveMCPSelections(servers: string[]): Promise<void> {
		this._storageService.store(MCP_SELECTIONS_KEY, JSON.stringify(servers), StorageScope.APPLICATION, StorageTarget.USER);
	}

	async getMCPSelections(): Promise<string[]> {
		const raw = this._storageService.get(MCP_SELECTIONS_KEY, StorageScope.APPLICATION);
		if (!raw) { return []; }
		try {
			return JSON.parse(raw) as string[];
		} catch {
			return [];
		}
	}

	private async _detectVSCodeConfig(): Promise<{ found: boolean; path: string | undefined }> {
		const home = env['HOME'];
		if (!home) {
			return { found: false, path: undefined };
		}
		const configUri = joinPath(URI.file(home), '.config', 'Code', 'User');
		try {
			const exists = await this._fileService.exists(configUri);
			return { found: exists, path: exists ? configUri.fsPath : undefined };
		} catch {
			return { found: false, path: undefined };
		}
	}

	private async _scanForApiKeys(): Promise<Record<string, string>> {
		const candidates: Array<{ envVar: string; providerId: string }> = [
			{ envVar: 'ANTHROPIC_API_KEY', providerId: 'anthropic' },
			{ envVar: 'OPENAI_API_KEY', providerId: 'openai' },
			{ envVar: 'GEMINI_API_KEY', providerId: 'gemini' },
		];

		const result: Record<string, string> = {};
		for (const { envVar, providerId } of candidates) {
			const value = env[envVar];
			if (value && value.length > 0) {
				result[providerId] = value;
			}
		}
		return result;
	}

	private async _probeLocalModels(): Promise<{ ollama: boolean; lmStudio: boolean }> {
		const [ollama, lmStudio] = await Promise.all([
			this._nativeHostService.probeLocalPort(11434),
			this._nativeHostService.probeLocalPort(1234),
		]);
		return { ollama, lmStudio };
	}

	private async _checkNpx(): Promise<boolean> {
		const pathEnv = env['PATH'];
		if (!pathEnv) {
			return false;
		}

		const dirs = pathEnv.split(':');
		for (const dir of dirs) {
			try {
				const exists = await this._fileService.exists(URI.file(`${dir}/npx`));
				if (exists) {
					return true;
				}
			} catch {
				// skip unreadable directories
			}
		}
		return false;
	}
}

registerSingleton(IForgeOnboardingService, ForgeOnboardingServiceImpl, InstantiationType.Delayed);
