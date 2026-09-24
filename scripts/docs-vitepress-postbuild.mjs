import { mkdir, readdir, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const root = fileURLToPath(new URL('../', import.meta.url))
const docs = path.join(root, 'docs')
const output = path.join(docs, '.vitepress', 'dist')
const entries = await readdir(docs, { withFileTypes: true })

for (const entry of entries) {
  if (!entry.isFile() || !entry.name.endsWith('.md') || entry.name === 'index.md') continue

  const slug = entry.name.slice(0, -3)
  const destination = path.join(output, slug)
  const target = `../${slug}`
  await mkdir(destination, { recursive: true })
  await writeFile(path.join(destination, 'index.html'), `<!doctype html>
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

await writeFile(path.join(output, '.nojekyll'), '')
