# R2: the public package repository and the private state bucket.
resource "cloudflare_r2_bucket" "repo" {
  account_id = var.account_id
  name       = "gnoblin-repo"
}

resource "cloudflare_r2_bucket" "state" {
  account_id = var.account_id
  name       = "gnoblin-tfstate"
}

# repo.gnoblin.org serves the repository bucket. The index Worker (infra/repo-index) runs on the same hostname.
resource "cloudflare_r2_custom_domain" "repo" {
  account_id  = var.account_id
  zone_id     = var.zone_id
  bucket_name = cloudflare_r2_bucket.repo.name
  domain      = "repo.gnoblin.org"
  enabled     = true
  min_tls     = "1.2"
}

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
