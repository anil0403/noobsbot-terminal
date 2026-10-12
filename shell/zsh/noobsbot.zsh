# noobsbot shell integration for zsh: marks prompts and commands (OSC 133)
# and reports the directory (OSC 7).

__noobsbot_precmd() {
    local s=$?
    print -n "\e]133;D;$s\a\e]7;file://$HOST$PWD\a"
    # Wrap last, after whatever the user's hooks did to the prompt.
    precmd_functions=(${precmd_functions:#__noobsbot_wrap} __noobsbot_wrap)
    return $s
}

__noobsbot_wrap() {
    [[ $PS1 == *'133;A'* ]] || PS1=$'%{\e]133;A\a%}'$PS1$'%{\e]133;B\a%}'
}

__noobsbot_preexec() {
    print -n "\e]133;C\a"
}

precmd_functions=(__noobsbot_precmd $precmd_functions)
preexec_functions+=(__noobsbot_preexec)

# Other machines won't know TERM=noobsbot.
[[ $TERM == noobsbot && -z ${aliases[ssh]} && -z ${functions[ssh]} ]] && ssh() { TERM=xterm-256color command ssh "$@" }
