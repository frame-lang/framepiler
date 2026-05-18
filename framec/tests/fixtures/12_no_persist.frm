@@[persist(String)]
@@[save(snapshot)]
@@[load(restore)]
@@system NoPersist {
    interface:
        bump()
        set_cache(v: i32)
        get_count(): i32
        get_cache(): i32

    machine:
        $Active {
            bump() { self.count = self.count + 1 }
            set_cache(v: i32) { self.cache = v }
            get_count(): i32 { @@:(self.count) }
            get_cache(): i32 { @@:(self.cache) }
        }

    domain:
        count: i32 = 0
        @@[no_persist]
        cache: i32 = -1
}
