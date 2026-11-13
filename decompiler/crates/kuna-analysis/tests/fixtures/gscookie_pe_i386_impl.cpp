struct Name { char text[16]; Name(const char *s); ~Name(); };
struct Sink { virtual void post(int to, int from, int msg, void *who, int flags) = 0; };
struct Unit { Sink *sink; int msg; float range; };
struct Log : Sink { int last; void post(int to, int from, int msg, void *who, int flags) { last = msg + flags; } };
void notify(Unit *u, void *who, float dist);
Name::Name(const char *s) { for (int i = 0; i < 15 && s[i]; i++) text[i] = s[i]; text[15] = 0; }
Name::~Name() { text[0] = 0; }
extern "C" int __cdecl start() { Log l; Unit u = { &l, 7, 3.0f }; notify(&u, 0, 1.0f); return l.last; }
