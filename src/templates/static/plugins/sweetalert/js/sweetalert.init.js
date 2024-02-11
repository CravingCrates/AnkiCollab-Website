  (document.querySelector(".sweet-success").onclick = function () {
    swal("Hey, Good job !!", "You clicked the button !!", "success");
  }),
  (document.querySelector(".sweet-success-cancel").onclick = function () {
    swal(
      {
        title: "Are you sure to delete ?",
        text: "You will not be able to recover this imaginary file !!",
        type: "warning",
        showCancelButton: !0,
        confirmButtonColor: "#DD6B55",
        confirmButtonText: "Yes, delete it !!",
        cancelButtonText: "No, cancel it !!",
        closeOnConfirm: !1,
        closeOnCancel: !1,
      },
      function (e) {
        e
          ? swal(
              "Deleted !!",
              "Hey, your imaginary file has been deleted !!",
              "success"
            )
          : swal(
              "Cancelled !!",
              "Hey, your imaginary file is safe !!",
              "error"
            );
      }
    );
  });
