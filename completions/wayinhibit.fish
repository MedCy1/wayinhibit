complete -c wayinhibit -f

complete -c wayinhibit -s t -l timeout \
    -d 'Stop after a given duration (e.g. 30s, 5m, 2h)' \
    -r -a '30s 1m 5m 10m 30m 1h 2h'
complete -c wayinhibit -s q -l quiet \
    -d 'Suppress informational output (warnings and errors remain)'
complete -c wayinhibit -s p -l pid-file \
    -d 'Write PID to PATH on startup and lock it, remove it on exit' \
    -r -F
complete -c wayinhibit -l on-inhibit \
    -d 'Run CMD (via sh -c) when inhibition starts' \
    -r
complete -c wayinhibit -l on-release \
    -d 'Run CMD (via sh -c) when inhibition stops' \
    -r
complete -c wayinhibit -l dry-run \
    -d 'Run without connecting to Wayland (for testing hooks)'
complete -c wayinhibit -l toggle \
    -d 'Stop the running instance from --pid-file, or start one'
complete -c wayinhibit -s h -l help \
    -d 'Print help'
complete -c wayinhibit -s V -l version \
    -d 'Print version'
