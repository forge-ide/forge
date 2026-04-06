import { Disposable } from '../../../../base/common/lifecycle.js';
import { URI } from '../../../../base/common/uri.js';
import { IFileService } from '../../../../platform/files/common/files.js';
import { InstantiationType, registerSingleton } from '../../../../platform/instantiation/common/extensions.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { IStorageService, StorageScope, StorageTarget } from '../../../../platform/storage/common/storage.js';
import { IEnvironmentDetectionResult, IForgeOnboardingService } from '../common/forgeOnboardingService.js';

const ONBOARDING_COMPLETE_KEY = 'forge.onboarding.complete';

export class ForgeOnboardingServiceImpl extends Disposable implements IForgeOnboardingService {
	declare readonly _serviceBrand: undefined;

	constructor(
		@IStorageService private readonly _storageService: IStorageService,
		@IFileService private readonly _fileService: IFileService,
		@ILogService private readonly _logService: ILogService,
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

	private async _detectVSCodeConfig(): Promise<{ found: boolean; path: string | undefined }> {
		const home = process.env['HOME'];
		if (!home) {
			return { found: false, path: undefined };
		}

		const configPath = `${home}/.config/Code/User`;
		try {
			const exists = await this._fileService.exists(URI.file(configPath));
			return { found: exists, path: exists ? configPath : undefined };
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
			const value = process.env[envVar];
			if (value && value.length > 0) {
				result[providerId] = value;
			}
		}
		return result;
	}

	private async _probeLocalModels(): Promise<{ ollama: boolean; lmStudio: boolean }> {
		const probe = async (url: string): Promise<boolean> => {
			const controller = new AbortController();
			const timer = setTimeout(() => controller.abort(), 500);
			try {
				await fetch(url, { signal: controller.signal, method: 'HEAD' });
				return true;
			} catch {
				return false;
			} finally {
				clearTimeout(timer);
			}
		};

		const [ollama, lmStudio] = await Promise.all([
			probe('http://localhost:11434'),
			probe('http://localhost:1234'),
		]);
		return { ollama, lmStudio };
	}

	private async _checkNpx(): Promise<boolean> {
		const pathEnv = process.env['PATH'];
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
