# noobsbot shell integration for bash, loaded with --rcfile in place of ~/.bashrc.
# Marks prompts and commands (OSC 133) and reports the directory (OSC 7).
[[ -f ~/.bashrc ]] && source ~/.bashrc

__noobsbot_status() {
    local s=$?
    __noobsbot_last=$s
    printf '\e]133;D;%s\a\e]7;file://%s%s\a' "$s" "$HOSTNAME" "$PWD"
    return $s
}

# Runs last, so it wraps whatever prompt the user's PROMPT_COMMAND built.
__noobsbot_wrap() {
    local s=$?
    [[ $PS1 == *'133;A'* ]] || PS1='\[\e]133;A\a\]'$PS1'\[\e]133;B\a\]'
    return $s
}

# The pretty prompt (terminal.pretty_prompt): powerline segments in palette
# colors, so themes recolor it. Rebuilt every time for the git branch and status.
__noobsbot_pretty() {
    local s=$? p='\[\e[38;5;8m\]╭─' prev='' branch bg=14 arrow=2
    # seg BG FG TEXT: an arrow from the previous segment, or a rounded start.
    __noobsbot_seg() {
        if [[ -z $prev ]]; then
            p+="\[\e[38;5;$1m\]"$'\ue0b6'"\[\e[48;5;$1;38;5;$2m\]"
        else
            p+="\[\e[38;5;${prev};48;5;$1m\]"$'\ue0b0'"\[\e[38;5;$2m\]"
        fi
        p+=" $3 "
        prev=$1
    }
    __noobsbot_seg 11 0 $'\uf007'" \u${SSH_CONNECTION:+@\h}"
    : "${PROMPT_DIRTRIM:=2}" # \w: ~/.../last/two
    __noobsbot_seg 12 15 $'\uf07b'" \w"
    if branch=$(git symbolic-ref --short HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null); then
        # Branch names may hold `$(...)`; PS1 must show them, not run them.
        branch=${branch//\\/\\\\}
        branch=${branch//\$/\\\$}
        branch=${branch//\`/\\\`}
        # ● when there's uncommitted work (no index lock: don't race the user's git).
        [[ -n $(GIT_OPTIONAL_LOCKS=0 git status --porcelain 2>/dev/null | head -c1) ]] && branch+=" ●" bg=208
        __noobsbot_seg $bg 0 $'\ue725'" $branch"
    fi
    # Failed: the exit code, and a red arrow.
    (( ${__noobsbot_last:-0} )) && __noobsbot_seg 1 15 $'\uf00d'" $__noobsbot_last" && arrow=1
    PS1=$p"\[\e[0;38;5;${prev}m\]"$'\ue0b4'"\n\[\e[38;5;8m\]╰─\[\e[38;5;${arrow}m\]❯\[\e[0m\] "
    return $s
}

# Array form needs bash 5.1+.
PROMPT_COMMAND=(__noobsbot_status "${PROMPT_COMMAND[@]}" __noobsbot_wrap)
[[ $NOOBSBOT_PROMPT == pretty ]] && PROMPT_COMMAND=("${PROMPT_COMMAND[@]:0:${#PROMPT_COMMAND[@]}-1}" __noobsbot_pretty __noobsbot_wrap)
PS0=$PS0'\e]133;C\a'

# Other machines won't know TERM=noobsbot.
[[ $TERM == noobsbot ]] && ! type -t ssh | grep -qE 'alias|function' && ssh() { TERM=xterm-256color command ssh "$@"; }
