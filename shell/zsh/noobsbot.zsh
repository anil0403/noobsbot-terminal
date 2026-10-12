# noobsbot shell integration for zsh: marks prompts and commands (OSC 133)
# and reports the directory (OSC 7).

__noobsbot_precmd() {
    local s=$?
    __noobsbot_last=$s
    print -n "\e]133;D;$s\a\e]7;file://$HOST$PWD\a"
    # Run last, after whatever the user's hooks did to the prompt.
    local last=(__noobsbot_wrap)
    [[ $NOOBSBOT_PROMPT == pretty ]] && last=(__noobsbot_pretty __noobsbot_wrap)
    # zsh runs a copy of the list, so the first prompt wouldn't see the change.
    # ponytail: on that first prompt, hooks from .zshrc still run after ours.
    [[ ${precmd_functions[-1]} == __noobsbot_wrap ]] || { for f in $last; do $f; done }
    precmd_functions=(${precmd_functions:#__noobsbot_(wrap|pretty)} $last)
    return $s
}

# The pretty prompt (terminal.pretty_prompt): powerline segments in palette
# colors, so themes recolor it. Rebuilt every time for the git branch and status.
__noobsbot_pretty() {
    local e=$'\e' p='' prev='' branch
    # seg BG FG TEXT: an arrow from the previous segment, or a rounded start.
    __noobsbot_seg() {
        if [[ -z $prev ]]; then
            p+="%{${e}[38;5;$1m%}"$'\ue0b6'"%{${e}[48;5;$1;38;5;$2m%}"
        else
            p+="%{${e}[38;5;${prev};48;5;$1m%}"$'\ue0b0'"%{${e}[38;5;$2m%}"
        fi
        p+=" $3 "
        prev=$1
    }
    __noobsbot_seg 11 0 "%n${SSH_CONNECTION:+@%m}"
    __noobsbot_seg 208 15 $'\uf07b'" %1~"
    if branch=$(git symbolic-ref --short HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null); then
        # Shown, never expanded: % is a prompt escape, and with prompt_subst so
        # is $(...), unless it comes from a parameter (expanded only once).
        __noobsbot_branch=${branch//\%/%%}
        [[ -o promptsubst ]] && branch='${__noobsbot_branch}' || branch=$__noobsbot_branch
        __noobsbot_seg 14 0 $'\ue725'" $branch"
    fi
    if (( ${__noobsbot_last:-0} == 0 )); then
        __noobsbot_seg 12 15 $'\uf00c'
    else
        __noobsbot_seg 1 15 $'\uf00d'" $__noobsbot_last"
    fi
    PS1=$p"%{${e}[0;38;5;${prev}m%}"$'\ue0b4'"%{${e}[0m%} "
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
