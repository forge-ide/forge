import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';

export const IForgeOnboardingService = createDecorator<IForgeOnboardingService>('forgeOnboardingService');

export interface IEnvironmentDetectionResult {
	hasVSCodeConfig: boolean;
	vscodeConfigPath: string | undefined;
	detectedApiKeys: Record<string, string>;
	ollamaRunning: boolean;
	lmStudioRunning: boolean;
	npxAvailable: boolean;
}

export interface IForgeOnboardingService {
	readonly _serviceBrand: undefined;
	readonly onboardingComplete: boolean;
	detectEnvironment(): Promise<IEnvironmentDetectionResult>;
	markComplete(): void;
	reset(): void;
}
