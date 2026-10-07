#!/usr/bin/env bash
# Guest: show how GeoClue is configured and which agent runs, to explain who answers location requests.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
echo "== geoclue version"
rpm -q geoclue2 2>&1 | head -1
echo "== agent whitelist from geoclue.conf and conf.d"
grep -h -A1 '^\[agent\]' /etc/geoclue/geoclue.conf /etc/geoclue/conf.d/*.conf 2>/dev/null
echo "== running agent processes"
pgrep -a -f 'geoclue-2.0/demos/agent' | cut -c1-140
echo "== who started the demo agent"
pid="$(pgrep -f 'geoclue-2.0/demos/agent' | head -1)"
if [ -n "$pid" ]; then
    echo "parent: $(ps -o ppid=,comm= -p "$(ps -o ppid= -p "$pid" | tr -d ' ')" 2>/dev/null)"
    echo "cgroup: $(cat /proc/"$pid"/cgroup 2>/dev/null | head -2 | tr '\n' ' ')"
fi
echo "== autostart entries that mention geoclue"
grep -l -i geoclue /etc/xdg/autostart/*.desktop 2>/dev/null
for file in /etc/xdg/autostart/geoclue-demo-agent.desktop; do
    [ -e "$file" ] && grep -v -E '^\s*(#|$)' "$file" | head -14
done
echo "== desktop identity seen by user services"
systemctl --user show-environment | grep -E '^(XDG_CURRENT_DESKTOP|XDG_SESSION_DESKTOP)='
echo "== user units that start xdg autostart apps"
systemctl --user list-units --no-pager --all 2>/dev/null | grep -i -E 'autostart|geoclue' | head -6 | cut -c1-140
