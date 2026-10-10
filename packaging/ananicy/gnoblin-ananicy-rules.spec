Name:           gnoblin-ananicy-rules
Version:        0.1.0
Release:        1%{?dist}
Summary:        Ananicy-cpp rules for the Gnoblin compositor and shell
License:        GPL-2.0-or-later
URL:            https://github.com/kierandrewett/gnoblin
Source0:        %{name}-%{version}.tar.gz
BuildArch:      noarch
Recommends:     ananicy-cpp
Supplements:    (gnoblin and ananicy-cpp)

%description
Ananicy-cpp's default rules rank gdm and its children as background work, with
nice 16, idle I/O and SCHED_IDLE. A Gnoblin session started by gdm inherits that
class, and the compositor and shell then freeze or drop frames while other
programs use the CPU.

This package adds rules that give the gnoblin, quickshell and bingux-frame
processes normal scheduling with a small priority boost. It contains only data
files for ananicy-cpp. It has no effect where ananicy-cpp is not installed.

%prep
%autosetup

%build

%install
install -Dpm644 99-gnoblin.types %{buildroot}%{_sysconfdir}/ananicy.d/99-gnoblin.types
install -Dpm644 99-gnoblin.rules %{buildroot}%{_sysconfdir}/ananicy.d/99-gnoblin.rules

%posttrans
systemctl try-restart ananicy-cpp.service >/dev/null 2>&1 || :

%postun
if [ $1 -eq 0 ]; then
    systemctl try-restart ananicy-cpp.service >/dev/null 2>&1 || :
fi

%files
%config(noreplace) %{_sysconfdir}/ananicy.d/99-gnoblin.types
%config(noreplace) %{_sysconfdir}/ananicy.d/99-gnoblin.rules

%changelog
* Sat Oct 10 2026 Kieran Drewett <kieran@drewett.dev> - 0.1.0-1
- Initial package
