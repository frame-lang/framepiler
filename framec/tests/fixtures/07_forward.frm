@@system Forward {
    interface:
        run()
        finish()

    machine:
        $Start {
            run() {
                self.starts = self.starts + 1
                -> => $Middle
            }
        }

        $Middle {
            run() { self.middles = self.middles + 1 }
            finish() { -> $End }
        }

        $End {
            finish() { }
        }

    domain:
        starts: i32 = 0
        middles: i32 = 0
}
