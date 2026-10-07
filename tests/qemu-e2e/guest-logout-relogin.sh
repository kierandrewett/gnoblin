#!/usr/bin/env bash
# Guest: after a logout, GDM shows the greeter. Restart it so the autologin starts the next session.
set -u
sudo systemctl restart gdm
