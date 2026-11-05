use sigmaos_dbscripts::*;

#[test]
fn component_is_constructible_and_runnable() {
    // Arrange
    let c = sigmaos_dbscripts::new("sigmaos-dbscripts").expect("valid name");
    // Act
    let out = c.run().expect("run succeeds");
    // Assert
    assert!(out.contains("sigmaos-dbscripts"));
}
