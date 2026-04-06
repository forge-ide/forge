import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';

export const IForgeVSCodeImportService = createDecorator<IForgeVSCodeImportService>('forgeVSCodeImportService');

export interface IVSCodeImportOptions {
	importSettings: boolean;
	importKeybindings: boolean;
	importExtensionsList: boolean;
	importApiKeys: boolean;
	sourcePath: string;
	detectedApiKeys: Record<string, string>; // provider id → key value (from env detection)
}

export interface IVSCodeImportResult {
	settingsImported: boolean;
	keybindingsImported: boolean;
	extensionsCount: number;
	apiKeysImported: string[]; // provider ids only — never log key values
	errors: string[];
}

export interface IForgeVSCodeImportService {
	readonly _serviceBrand: undefined;
	import(options: IVSCodeImportOptions): Promise<IVSCodeImportResult>;
}
