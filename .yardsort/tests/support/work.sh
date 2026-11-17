#!/bin/sh
# An agent at work, for notifications-there.yaml: ten seconds of printing, which is a stretch of
# work to whatever watches its terminal, and then it waits, as an agent does when it is done.
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
  echo "working $i"
  sleep 0.5
done
echo "done: $*"
sleep 3600
