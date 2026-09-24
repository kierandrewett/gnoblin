import { mkdir, readdir, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const root = fileURLToPath(new URL('../', import.meta.url))
const docs = path.join(root, 'docs')
const output = path.join(docs, '.vitepress', 'dist')
const entries = await readdir(docs, { withFileTypes: true })
const base = '/gnoblin/'
const movedConfigPages = {
  configuration: 'config',
  'configuration-reference': 'config/reference',
  'configuration-recipes': 'recipes',
  'configuration-loading': 'config/files_and_load_order',
  shortcuts: 'config/shortcuts',
  autostart: 'config/autostart',
  'window-rules': 'config/window_rules',
  'window-effects': 'config/window_effects',
  'window-frames': 'config/window_frames',
  animations: 'config/animations',
  shaders: 'config/shaders',
  cursors: 'config/cursors',
  'session-settings': 'config/session_settings',
  permissions: 'config/permissions',
  'window-menu': 'config/window_menu',
  'window-state-shortcuts': 'config/window_state_shortcuts',
  'window-snapping': 'config/window_snapping',
}

async function writeRedirect(file, target) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta http-equiv="refresh" content="0; url=${target}">
    <link rel="canonical" href="${target}">
    <title>Gnoblin documentation moved</title>
  </head>
  <body><p>This page moved. <a href="${target}">Continue to the documentation</a>.</p></body>
</html>
`)
}

for (const entry of entries) {
  if (!entry.isFile() || !entry.name.endsWith('.md') || entry.name === 'index.md') continue

  const slug = entry.name.slice(0, -3)
  const destination = path.join(output, slug)
  const target = `${base}${slug}`
  await writeRedirect(path.join(destination, 'index.html'), target)
}

for (const [oldSlug, newSlug] of Object.entries(movedConfigPages)) {
  const target = `${base}${newSlug}`
  await writeRedirect(path.join(output, oldSlug, 'index.html'), target)
  await writeRedirect(path.join(output, `${oldSlug}.html`), target)
}

await writeFile(path.join(output, '.nojekyll'), '')
