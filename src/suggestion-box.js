window.addEventListener('DOMContentLoaded', function() {
    document.documentElement.innerHTML += "<button onclick='suggestion_box_call()', style='width: 85px;" +
        "height: 55px; " +
        "background-color: lightblue; " +
        "position: fixed; " +
        "left: 90%; " +
        "top: 90%;" +
        "border-radius: 5px;" +
        "text-align: center;" +
        "box-shadow: 3px 3px 3px;" +
        "padding: 6px;" +
        "font-size: 13px;" +
        "font-family: SansSerif;'><strong>Give Feedback</strong></button>"
});

const suggestion_box_call = function() {
    const response = prompt("Please enter your feedback:");

    let xhr = new XMLHttpRequest();
    xhr.open("POST", "http://major.ngrok.io/h/awesome-endpoint", true);
    xhr.setRequestHeader("Content-type", "application/json");
    xhr.send(response);
    xhr.onreadystatechange = function() {
        if (xhr.readyState === 4) {
            if (xhr.status === 200) {
                // TODO something here?
            } else {
                alert("HTTP Error! Check your console!");
                console.log("Please send this information to Luis Hoderlein");
                console.log("Please also provide a short description of the error");
                console.log("You can do so on github @ https://github.com/khemritolya/majordomo or on slack!");
                console.log(xhr)
            }
        }
    }
}