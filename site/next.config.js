// GitHub Pages serves the site from https://lussoluca.github.io/phplumb/.
const basePath = '/phplumb'

/** @type {import('next').NextConfig} */
const nextConfig = {
  output: 'export',
  basePath,
  images: { unoptimized: true },
  env: { NEXT_PUBLIC_BASE_PATH: basePath },
}

module.exports = nextConfig
