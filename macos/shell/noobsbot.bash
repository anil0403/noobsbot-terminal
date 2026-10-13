# noobsbot shell integration for bash, loaded with --rcfile. Marks prompts and
# commands (OSC 133) and reports the directory (OSC 7). Works on macOS's own
# bash 3.2 too (no \u escapes, PS0 or PROMPT_COMMAND arrays there).
# A login shell, as Terminal.app starts: /etc/profile puts the system paths on
# PATH (path_helper), then the first of your profiles (Homebrew sets up there).
[[ -f /etc/profile ]] && source /etc/profile
for __noobsbot_f in ~/.bash_profile ~/.bash_login ~/.profile; do
    [[ -f $__noobsbot_f ]] && { source "$__noobsbot_f"; break; }
done
unset __noobsbot_f

__noobsbot_status() {
    local s=$?
    __noobsbot_last=$s
    printf '\e]133;D;%s\a\e]7;file://%s%s\a' "$s" "$HOSTNAME" "$PWD"
    return $s
}

# Runs last, so it wraps whatever prompt the user's PROMPT_COMMAND built.
__noobsbot_wrap() {
    local s=$?
    [[ ${PS1-} == *'133;A'* ]] || PS1='\[\e]133;A\a\]'${PS1-}'\[\e]133;B\a\]'
    __noobsbot_armed=1
    return $s
}

# The pretty prompt (terminal.pretty_prompt): powerline segments in palette
# colors, so themes recolor it. Rebuilt every time for the git branch and status.
__noobsbot_pretty() {
    local s=$? p='' prev='' branch
    # A blank line between prompts (not above the first), so their pills don't touch.
    [[ -n ${__noobsbot_gap-} ]] && echo
    __noobsbot_gap=1
    # seg BG FG TEXT: an arrow from the previous segment, or a rounded start.
    __noobsbot_seg() {
        if [[ -z $prev ]]; then
            p+="\[\e[38;5;$1m\]"$'\xee\x82\xb6'"\[\e[48;5;$1;38;5;$2m\]"
        else
            p+="\[\e[38;5;${prev};48;5;$1m\]"$'\xee\x82\xb0'"\[\e[38;5;$2m\]"
        fi
        p+=" $3 "
        prev=$1
    }
    __noobsbot_seg 11 0 "\u${SSH_CONNECTION:+@\h}"
    __noobsbot_seg 208 15 $'\xef\x81\xbb'" \W"
    # One git run (no index lock: don't race the user's git): "# branch.oid SHA",
    # "# branch.head NAME" (or "(detached)"), then a line per change.
    local st; st=$(GIT_OPTIONAL_LOCKS=0 git status --porcelain=v2 --branch 2>/dev/null)
    if [[ -n $st ]]; then
        branch=${st#*$'# branch.head '} branch=${branch%%$'\n'*}
        [[ $branch == '(detached)' ]] && branch=${st:13:7}
        # Branch names may hold `$(...)`; PS1 must show them, not run them.
        branch=${branch//\\/\\\\}
        branch=${branch//\$/\\\$}
        branch=${branch//\`/\\\`}
        # ● when there's uncommitted work.
        [[ $st == *$'\n'[!#]* ]] && branch+=" ●"
        __noobsbot_seg 14 0 $'\xee\x9c\xa5'" $branch"
    fi
    if (( ${__noobsbot_last:-0} == 0 )); then
        __noobsbot_seg 12 15 $'\xef\x80\x8c'
    else
        __noobsbot_seg 1 15 $'\xef\x80\x8d'" $__noobsbot_last"
    fi
    PS1=$p"\[\e[0;38;5;${prev}m\]"$'\xee\x82\xb4'"\[\e[0m\] "
    return $s
}

__noobsbot_last_cmd=__noobsbot_wrap
[[ ${NOOBSBOT_PROMPT-} == pretty ]] && __noobsbot_last_cmd='__noobsbot_pretty; __noobsbot_wrap'
if (( BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1) )); then
    PROMPT_COMMAND=(__noobsbot_status "${PROMPT_COMMAND[@]}" "$__noobsbot_last_cmd")
else
    PROMPT_COMMAND="__noobsbot_status; ${PROMPT_COMMAND:+$PROMPT_COMMAND; }$__noobsbot_last_cmd"
fi
unset __noobsbot_last_cmd
if (( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) )); then
    PS0=${PS0-}'\e]133;C\a'
elif [[ -z $(trap -p DEBUG) ]]; then
    # No PS0 before bash 4.4: mark the command's start from the DEBUG trap, once
    # per prompt (__noobsbot_wrap arms it as the prompt's last step; functions
    # don't trip the trap). An empty line goes straight to __noobsbot_status: no mark.
    trap '[[ -n ${__noobsbot_armed-} ]] && { __noobsbot_armed=; [[ $BASH_COMMAND == __noobsbot_status* ]] || printf "\e]133;C\a"; }' DEBUG
fi

# Other machines won't know TERM=noobsbot. An `if`, not `&&`: this is the
# rcfile's last line, and its status would show on the first prompt.
if [[ $TERM == noobsbot ]] && ! type -t ssh | grep -qE 'alias|function'; then
    ssh() { TERM=xterm-256color command ssh "$@"; }
fi
