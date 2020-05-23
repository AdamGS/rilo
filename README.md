# rilo
writing [kilo](http://antirez.com/news/108) in rust, loosely following [this](https://viewsourcecode.org/snaptoken/kilo/index.html) tutorial.

---

**WARNING:** rilo may make your terminal behave kina wierd, as it is only tested on Ubuntu and a pretty standard configuration

### TODOs:
- The whole main loop is getting a bit too noisy, there is probably a better way to do it (maybe a "run" function?, maybe split it into some input_loop with a callback)
- Do I want a better way to handle incoming input? some struct over stdin.
