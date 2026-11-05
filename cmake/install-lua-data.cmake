# Installs the shipped Lua modules under <INSTALL_PREFIX>/share/gnoblin/lua so
# they load as require("gnoblin.layouts.<name>") from that root.
#
# Expects, from the including script: ROOT (repository root) and INSTALL_PREFIX
# (absolute install prefix). Included by install-session.cmake and run on its
# own by the install-path test.
file(COPY "${ROOT}/src/data/lua/" DESTINATION "${INSTALL_PREFIX}/share/gnoblin/lua"
  USE_SOURCE_PERMISSIONS)
