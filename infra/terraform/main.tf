# R2: the public package repository and the private state bucket.
resource "cloudflare_r2_bucket" "repo" {
  account_id = var.account_id
  name       = "gnoblin-repo"
}

resource "cloudflare_r2_bucket" "state" {
  account_id = var.account_id
  name       = "gnoblin-tfstate"
}

# The R2 custom domain repo.gnoblin.org (bucket gnoblin-repo, minimum TLS 1.2) is not managed here. The provider cannot
# import cloudflare_r2_custom_domain, and creating it would conflict with the live domain.

# Mail records. gnoblin.org sends no mail, so these reject spoofed mail.
resource "cloudflare_dns_record" "dmarc" {
  zone_id = var.zone_id
  name    = "_dmarc.gnoblin.org"
  type    = "TXT"
  ttl     = 1
  proxied = false
  content = "\"v=DMARC1; p=reject; sp=reject; adkim=s; aspf=s;\""
}

resource "cloudflare_dns_record" "dkim_wildcard" {
  zone_id = var.zone_id
  name    = "*._domainkey.gnoblin.org"
  type    = "TXT"
  ttl     = 1
  proxied = false
  content = "\"v=DKIM1; p=\""
}

resource "cloudflare_dns_record" "spf" {
  zone_id = var.zone_id
  name    = "gnoblin.org"
  type    = "TXT"
  ttl     = 1
  proxied = false
  content = "\"v=spf1 -all\""
}

# Repository index Worker. It lists bucket folders. Everything else passes through to the R2 custom domain.
# CI builds infra/repo-index/dist/index.js before Terraform runs.
resource "cloudflare_workers_script" "repo_index" {
  account_id         = var.account_id
  script_name        = "repo-index"
  main_module        = "index.js"
  content_file       = "${path.module}/../repo-index/dist/index.js"
  content_sha256     = filesha256("${path.module}/../repo-index/dist/index.js")
  compatibility_date = "2026-10-01"

  bindings = [{
    name        = "REPO"
    type        = "r2_bucket"
    bucket_name = cloudflare_r2_bucket.repo.name
  }]
}

data "cloudflare_workers_routes" "all" {
  zone_id = var.zone_id
}

resource "cloudflare_workers_route" "repo_index" {
  zone_id = var.zone_id
  pattern = "repo.gnoblin.org/*"
  script  = cloudflare_workers_script.repo_index.script_name
}

# Website Worker. It serves the VitePress build as static assets. CI builds infra/website/.vitepress/dist, adds the
# APT archive and writes _redirects before Terraform runs.
resource "cloudflare_workers_script" "website" {
  account_id         = var.account_id
  script_name        = "gnoblin-website"
  main_module        = "worker.js"
  content            = "export default { fetch: (request, env) => env.ASSETS.fetch(request) };"
  compatibility_date = "2026-10-01"

  assets = {
    directory = "${path.module}/../website/.vitepress/dist"
    config = {
      html_handling      = "auto-trailing-slash"
      not_found_handling = "404-page"
      redirects          = file("${path.module}/../website/.vitepress/dist/_redirects")
    }
  }

  bindings = [{
    name = "ASSETS"
    type = "assets"
  }]
}

resource "cloudflare_workers_custom_domain" "website" {
  for_each   = toset(["gnoblin.org", "www.gnoblin.org"])
  account_id = var.account_id
  zone_id    = var.zone_id
  hostname   = each.key
  service    = cloudflare_workers_script.website.script_name
}
