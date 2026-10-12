# noobsbot shell integration for bash, loaded with --rcfile in place of ~/.bashrc.
# Marks prompts and commands (OSC 133) and reports the directory (OSC 7).
[[ -f ~/.bashrc ]] && source ~/.bashrc

__noobsbot_status() {
    local s=$?
    printf '\e]133;D;%s\a\e]7;file://%s%s\a' "$s" "$HOSTNAME" "$PWD"
    return $s
}

# Runs last, so it wraps whatever prompt the user's PROMPT_COMMAND built.
__noobsbot_wrap() {
    local s=$?
    [[ $PS1 == *'133;A'* ]] || PS1='\[\e]133;A\a\]'$PS1'\[\e]133;B\a\]'
    return $s
}

# Array form needs bash 5.1+.
PROMPT_COMMAND=(__noobsbot_status "${PROMPT_COMMAND[@]}" __noobsbot_wrap)
PS0=$PS0'\e]133;C\a'
