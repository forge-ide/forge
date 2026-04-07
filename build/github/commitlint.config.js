export default {
	extends: ['@commitlint/config-conventional'],
	rules: {
		'type-enum': [2, 'always', [
			'feat', 'fix', 'perf', 'revert',
			'docs', 'style', 'chore', 'test',
			'refactor', 'ci', 'build'
		]],
		'subject-case': [0],
	},
};
