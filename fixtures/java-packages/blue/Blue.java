package blue;

import red.Red;

public class Blue {
    public String name() {
        return "blue+" + new Red().describe();
    }
}
