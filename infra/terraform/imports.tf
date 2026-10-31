# Resources that existed before Terraform. Each block is a no-op once the resource is in the state.
import {
  to = cloudflare_r2_bucket.repo
  id = "${var.account_id}/gnoblin-repo/default"
}

import {
  to = cloudflare_r2_bucket.state
  id = "${var.account_id}/gnoblin-tfstate/default"
}

import {
  to = cloudflare_r2_custom_domain.repo
  id = "${var.account_id}/gnoblin-repo/repo.gnoblin.org"
}

import {
  to = cloudflare_dns_record.dmarc
  id = "${var.zone_id}/2aab56a35035f97cb6a32f7298cc698e"
}

import {
  to = cloudflare_dns_record.dkim_wildcard
  id = "${var.zone_id}/e1221aa74208860f86a55a4a706ba4a7"
}

import {
  to = cloudflare_dns_record.spf
  id = "${var.zone_id}/5bc2c291e903ddf43f4c9bffe68fa89f"
}
