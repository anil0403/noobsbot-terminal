# noobsbot: zsh was started with ZDOTDIR pointing here. Put the user's back,
# load their .zshenv (zsh then reads their .zshrc), and hook in.
if [[ -n $NOOBSBOT_ZDOTDIR ]]; then ZDOTDIR=$NOOBSBOT_ZDOTDIR; else unset ZDOTDIR; fi
unset NOOBSBOT_ZDOTDIR
[[ -f ${ZDOTDIR:-$HOME}/.zshenv ]] && source ${ZDOTDIR:-$HOME}/.zshenv
[[ -o interactive ]] && source ${${(%):-%x}:h}/noobsbot.zsh
