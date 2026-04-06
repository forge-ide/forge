import { Disposable } from '../../../../base/common/lifecycle.js';
import { joinPath } from '../../../../base/common/resources.js';
import { URI } from '../../../../base/common/uri.js';
import { IEnvironmentService } from '../../../../platform/environment/common/environment.js';
import { IFileService } from '../../../../platform/files/common/files.js';
import { InstantiationType, registerSingleton } from '../../../../platform/instantiation/common/extensions.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { ISecretStorageService } from '../../../../platform/secrets/common/secrets.js';
import { IForgeVSCodeImportService, IVSCodeImportOptions, IVSCodeImportResult } from '../common/forgeVSCodeImportService.js';

export class ForgeVSCodeImportServiceImpl extends Disposable implements IForgeVSCodeImportService {
	readonly _serviceBrand: undefined;

	constructor(
		@IFileService private readonly fileService: IFileService,
		@ILogService private readonly logService: ILogService,
		@ISecretStorageService private readonly secretStorageService: ISecretStorageService,
		@IEnvironmentService private readonly environmentService: IEnvironmentService,
	) {
		super();
	}

	async import(options: IVSCodeImportOptions): Promise<IVSCodeImportResult> {
		const errors: string[] = [];
		let settingsImported = false;
		let keybindingsImported = false;
		let extensionsCount = 0;
		const apiKeysImported: string[] = [];

		if (options.importSettings && options.sourcePath) {
			try {
				const source = URI.file(options.sourcePath + '/settings.json');
				const dest = joinPath(joinPath(this.environmentService.userRoamingDataHome, 'User'), 'settings.json');
				await this.fileService.copy(source, dest, true);
				settingsImported = true;
				this.logService.info('[ForgeVSCodeImportService] settings.json imported');
			} catch (err) {
				const msg = err instanceof Error ? err.message : String(err);
				this.logService.warn('[ForgeVSCodeImportService] Failed to import settings.json: ' + msg);
				errors.push('Failed to import settings.json: ' + msg);
			}
		}

		if (options.importKeybindings && options.sourcePath) {
			try {
				const source = URI.file(options.sourcePath + '/keybindings.json');
				const dest = joinPath(joinPath(this.environmentService.userRoamingDataHome, 'User'), 'keybindings.json');
				await this.fileService.copy(source, dest, true);
				keybindingsImported = true;
				this.logService.info('[ForgeVSCodeImportService] keybindings.json imported');
			} catch (err) {
				const msg = err instanceof Error ? err.message : String(err);
				this.logService.warn('[ForgeVSCodeImportService] Failed to import keybindings.json: ' + msg);
				errors.push('Failed to import keybindings.json: ' + msg);
			}
		}

		if (options.importExtensionsList && options.sourcePath) {
			try {
				const source = URI.file(options.sourcePath + '/extensions.json');
				const content = await this.fileService.readFile(source);
				const parsed: unknown = JSON.parse(content.value.toString());
				if (Array.isArray(parsed)) {
					extensionsCount = parsed.length;
				} else {
					errors.push('extensions.json is not an array');
				}
				this.logService.info('[ForgeVSCodeImportService] extensions.json read, count=' + extensionsCount);
			} catch (err) {
				const msg = err instanceof Error ? err.message : String(err);
				this.logService.warn('[ForgeVSCodeImportService] Failed to read extensions.json: ' + msg);
				errors.push('Failed to read extensions.json: ' + msg);
			}
		}

		if (options.importApiKeys) {
			for (const [providerId, keyValue] of Object.entries(options.detectedApiKeys)) {
				try {
					await this.secretStorageService.set('forge.apikey.' + providerId, keyValue);
					apiKeysImported.push(providerId);
					this.logService.info('[ForgeVSCodeImportService] API key stored for provider: ' + providerId);
				} catch (err) {
					const msg = err instanceof Error ? err.message : String(err);
					this.logService.warn('[ForgeVSCodeImportService] Failed to store API key for provider ' + providerId + ': ' + msg);
					errors.push('Failed to store API key for provider ' + providerId + ': ' + msg);
				}
			}
		}

		return { settingsImported, keybindingsImported, extensionsCount, apiKeysImported, errors };
	}
}

registerSingleton(IForgeVSCodeImportService, ForgeVSCodeImportServiceImpl, InstantiationType.Delayed);
