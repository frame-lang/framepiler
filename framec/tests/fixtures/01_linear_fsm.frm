@@system LinearFsm {
    interface:
        start()
        progress(amount: i32)
        finish()

    machine:
        $Idle {
            start() { -> $Active }
        }

        $Active {
            progress(amount: i32) {
                // self mutation exercised below via domain field
                self.total = self.total + amount
            }
            finish() { -> $Done }
        }

        $Done { }

    domain:
        total: i32 = 0
}
