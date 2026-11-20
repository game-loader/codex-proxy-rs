import antfu from '@antfu/eslint-config'

export default antfu({
  type: 'app',
  typescript: true,
  vue: { a11y: true },
  ignores: ['dist/**', 'pnpm-workspace.yaml'],
})
