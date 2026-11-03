package green;

import red.Red;

public class Green {
    public String mix() {
        return "green+" + new Red().describe();
    }
}
