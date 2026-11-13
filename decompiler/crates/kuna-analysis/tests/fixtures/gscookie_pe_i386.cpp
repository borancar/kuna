struct Name { char text[16]; Name(const char *s); ~Name(); };
struct Sink { virtual void post(int to, int from, int msg, void *who, int flags) = 0; };
struct Unit { Sink *sink; int msg; float range; };
void notify(Unit *u, void *who, float dist) {
    Name tag("MSG_WAIT");
    int flags = dist < u->range;
    if (tag.text[0] == 'M') flags |= 8;
    u->sink->post(-1, -1, u->msg, who, flags);
}
