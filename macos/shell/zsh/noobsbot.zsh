# noobsbot shell integration for zsh: marks prompts and commands (OSC 133)
# and reports the directory (OSC 7).

__noobsbot_precmd() {
    local s=$?
    __noobsbot_last=$s
    print -n "\e]133;D;$s\a\e]7;file://$HOST$PWD\a"
    # Run last, after whatever the user's hooks did to the prompt.
    local last=(__noobsbot_wrap)
    [[ ${NOOBSBOT_PROMPT-} == pretty ]] && last=(__noobsbot_pretty __noobsbot_wrap)
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
    # A blank line between prompts (not above the first), so their pills don't touch.
    [[ -n ${__noobsbot_gap-} ]] && print
    __noobsbot_gap=1
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
    # One git run (no index lock: don't race the user's git): "# branch.oid SHA",
    # "# branch.head NAME" (or "(detached)"), then a line per change.
    local st=$(GIT_OPTIONAL_LOCKS=0 git status --porcelain=v2 --branch 2>/dev/null)
    if [[ -n $st ]]; then
        branch=${st#*$'# branch.head '} branch=${branch%%$'\n'*}
        [[ $branch == '(detached)' ]] && branch=${st:13:7}
        # Shown, never expanded: % is a prompt escape, and with prompt_subst so
        # is $(...), unless it comes from a parameter (expanded only once).
        __noobsbot_branch=${branch//\%/%%}
        [[ -o promptsubst ]] && branch='${__noobsbot_branch}' || branch=$__noobsbot_branch
        # ● when there's uncommitted work.
        [[ $st == *$'\n'[^#]* ]] && branch+=" ●"
        __noobsbot_seg 14 0 $'\ue725'" $branch"
    fi
    # 130 is Ctrl-C (also at an empty prompt): not a failure.
    if (( ${__noobsbot_last:-0} == 0 || __noobsbot_last == 130 )); then
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

typeset -ga precmd_functions
precmd_functions=(__noobsbot_precmd $precmd_functions)
preexec_functions+=(__noobsbot_preexec)

# Other machines won't know TERM=noobsbot.
[[ $TERM == noobsbot && -z ${aliases[ssh]-} && -z ${functions[ssh]-} ]] && ssh() { TERM=xterm-256color command ssh "$@" }
